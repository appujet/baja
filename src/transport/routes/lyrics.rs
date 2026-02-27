use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    response::IntoResponse,
};

use crate::{
    protocol::{
        models::{
            GetLyricsQuery, GetPlayerLyricsQuery, LyricsLoadResult,
            LyricsResultData as ApiLyricsData, RustalinkLyrics, RustalinkLyricsLine,
        },
        tracks::Track,
    },
    server::AppState,
};

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadLyricsQuery {
    pub encoded_track: String,
    pub lang: Option<String>,
}

pub async fn load_lyrics(
    State(state): State<Arc<AppState>>,
    Query(query): Query<LoadLyricsQuery>,
) -> Json<LyricsLoadResult> {
    tracing::info!(
        "GET /v4/loadlyrics: encoded_track='{}', lang={:?}",
        query.encoded_track,
        query.lang
    );
    let track = match Track::decode(&query.encoded_track) {
        Some(t) => t,
        None => {
            return Json(LyricsLoadResult::Error(
                crate::protocol::models::LyricsLoadError {
                    message: "Invalid encoded track".to_string(),
                    severity: crate::common::Severity::Common,
                },
            ));
        }
    };

    match state.lyrics_manager.load_lyrics(&track.info).await {
        Some(lyrics) => {
            if let Some(lines) = lyrics.lines {
                Json(LyricsLoadResult::Lyrics(ApiLyricsData {
                    name: lyrics.name,
                    synced: true,
                    lines,
                }))
            } else {
                Json(LyricsLoadResult::Text(
                    crate::protocol::models::LyricsTextData { text: lyrics.text },
                ))
            }
        }
        None => Json(LyricsLoadResult::Empty {}),
    }
}

pub async fn subscribe_lyrics(
    State(state): State<Arc<AppState>>,
    Path((session_id, guild_id)): Path<(String, String)>,
) -> axum::http::StatusCode {
    let session_id = crate::common::types::SessionId(session_id);
    let guild_id = crate::common::types::GuildId(guild_id);
    tracing::info!(
        "POST /v4/sessions/{}/players/{}/lyrics/subscribe",
        session_id,
        guild_id
    );

    if let Some(session) = state.sessions.get(&session_id) {
        if let Some(player_ref) = session.players.get(&guild_id) {
            let player: &crate::player::PlayerContext = player_ref.value();
            player.subscribe_lyrics().await;

            if let Some(track) = &player.track_info {
                let lyrics_data_arc = player.lyrics_data.clone();
                let lyrics_manager_clone = state.lyrics_manager.clone();
                let track_info_clone = track.info.clone();
                let session_lyrics_clone = session.clone();
                let guild_id_lyrics = player.guild_id.clone();

                tokio::spawn(async move {
                    let has_lyrics = lyrics_data_arc.lock().await.is_some();
                    if !has_lyrics {
                        if let Some(lyrics) =
                            lyrics_manager_clone.load_lyrics(&track_info_clone).await
                        {
                            {
                                let mut lock = lyrics_data_arc.lock().await;
                                *lock = Some(lyrics.clone());
                            }
                            let event = crate::protocol::OutgoingMessage::Event(
                                crate::protocol::RustalinkEvent::LyricsFound {
                                    guild_id: guild_id_lyrics,
                                    lyrics: crate::protocol::models::RustalinkLyrics {
                                        source_name: track_info_clone.source_name.clone(),
                                        provider: Some(lyrics.provider),
                                        text: Some(lyrics.text),
                                        lines: lyrics.lines.map(|lines| {
                                            lines
                                                .into_iter()
                                                .map(|l| {
                                                    crate::protocol::models::RustalinkLyricsLine {
                                                        timestamp: l.timestamp,
                                                        duration: Some(l.duration),
                                                        line: l.text,
                                                        plugin: serde_json::json!({}),
                                                    }
                                                })
                                                .collect()
                                        }),
                                        plugin: serde_json::json!({}),
                                    },
                                },
                            );
                            session_lyrics_clone.send_message(&event);
                        } else {
                            let event = crate::protocol::OutgoingMessage::Event(
                                crate::protocol::RustalinkEvent::LyricsNotFound {
                                    guild_id: guild_id_lyrics,
                                },
                            );
                            session_lyrics_clone.send_message(&event);
                        }
                    }
                });
            }

            return axum::http::StatusCode::NO_CONTENT;
        }
    }
    axum::http::StatusCode::NOT_FOUND
}

pub async fn unsubscribe_lyrics(
    State(state): State<Arc<AppState>>,
    Path((session_id, guild_id)): Path<(String, String)>,
) -> axum::http::StatusCode {
    let session_id = crate::common::types::SessionId(session_id);
    let guild_id = crate::common::types::GuildId(guild_id);
    tracing::info!(
        "POST /v4/sessions/{}/players/{}/lyrics/unsubscribe",
        session_id,
        guild_id
    );

    if let Some(session) = state.sessions.get(&session_id) {
        if let Some(player_ref) = session.players.get(&guild_id) {
            let player: &crate::player::PlayerContext = player_ref.value();
            player.unsubscribe_lyrics().await;
            return axum::http::StatusCode::NO_CONTENT;
        }
    }
    axum::http::StatusCode::NOT_FOUND
}

pub async fn get_lyrics(
    State(state): State<Arc<AppState>>,
    Query(query): Query<GetLyricsQuery>,
) -> impl IntoResponse {
    tracing::info!(
        "GET /v4/lyrics: track='{}', skipTrackSource={}",
        query.track,
        query.skip_track_source
    );
    let track = match Track::decode(&query.track) {
        Some(t) => t,
        None => {
            return (axum::http::StatusCode::BAD_REQUEST, "Invalid encoded track").into_response();
        }
    };

    match state
        .lyrics_manager
        .load_lyrics_ext(&track.info, query.skip_track_source)
        .await
    {
        Some(lyrics) => {
            let response = RustalinkLyrics {
                source_name: track.info.source_name.clone(),
                provider: Some(lyrics.provider),
                text: Some(lyrics.text),
                lines: lyrics
                    .lines
                    .map(|lines: Vec<crate::protocol::models::LyricsLine>| {
                        lines
                            .into_iter()
                            .map(|l| RustalinkLyricsLine {
                                timestamp: l.timestamp,
                                duration: Some(l.duration),
                                line: l.text,
                                plugin: serde_json::json!({}),
                            })
                            .collect()
                    }),
                plugin: serde_json::json!({}),
            };
            Json(response).into_response()
        }
        None => axum::http::StatusCode::NO_CONTENT.into_response(),
    }
}

pub async fn get_player_lyrics(
    State(state): State<Arc<AppState>>,
    Path((session_id, guild_id)): Path<(String, String)>,
    Query(query): Query<GetPlayerLyricsQuery>,
) -> impl IntoResponse {
    let session_id = crate::common::types::SessionId(session_id);
    let guild_id = crate::common::types::GuildId(guild_id);
    tracing::info!(
        "GET /v4/sessions/{}/players/{}/track/lyrics: skipTrackSource={}",
        session_id,
        guild_id,
        query.skip_track_source
    );

    let session = match state.sessions.get(&session_id) {
        Some(s) => s,
        None => return axum::http::StatusCode::NOT_FOUND.into_response(),
    };

    let player = match session.players.get(&guild_id) {
        Some(p) => p,
        None => return axum::http::StatusCode::NOT_FOUND.into_response(),
    };

    let track = match &player.track_info {
        Some(t) => t,
        None => return axum::http::StatusCode::NOT_FOUND.into_response(),
    };

    match state
        .lyrics_manager
        .load_lyrics_ext(&track.info, query.skip_track_source)
        .await
    {
        Some(lyrics) => {
            let response = RustalinkLyrics {
                source_name: track.info.source_name.clone(),
                provider: Some(lyrics.provider),
                text: Some(lyrics.text),
                lines: lyrics
                    .lines
                    .map(|lines: Vec<crate::protocol::models::LyricsLine>| {
                        lines
                            .into_iter()
                            .map(|l| RustalinkLyricsLine {
                                timestamp: l.timestamp,
                                duration: Some(l.duration),
                                line: l.text,
                                plugin: serde_json::json!({}),
                            })
                            .collect()
                    }),
                plugin: serde_json::json!({}),
            };
            Json(response).into_response()
        }
        None => axum::http::StatusCode::NO_CONTENT.into_response(),
    }
}
