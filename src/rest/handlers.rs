use crate::rest::models::*;
use crate::server::{AppState, PlayerContext};
use crate::sources::SourceManager;
use crate::types;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Json},
};
use std::sync::Arc;
use tracing::info;

/// GET /v4/loadtracks?identifier=...
pub async fn load_tracks(
    Query(params): Query<LoadTracksQuery>,
    State(_state): State<Arc<AppState>>,
) -> Json<types::LoadResult> {
    let identifier = params.identifier;
    tracing::debug!("Load tracks: '{}'", identifier);

    let source_manager = SourceManager::new();
    let response = source_manager.load(&identifier).await;

    // Convert legacy LoadTracksResponse to types::LoadResult
    Json(match response.load_type {
        LoadType::Track => {
            if let LoadData::Track(t) = response.data {
                types::LoadResult::Track(types::Track {
                    encoded: t.encoded,
                    info: types::TrackInfo {
                        identifier: t.info.identifier,
                        is_seekable: t.info.is_seekable,
                        author: t.info.author,
                        length: t.info.length,
                        is_stream: t.info.is_stream,
                        position: t.info.position,
                        title: t.info.title,
                        uri: Some(t.info.uri),
                        artwork_url: t.info.artwork_url,
                        isrc: t.info.isrc,
                        source_name: t.info.source_name,
                    },
                    plugin_info: serde_json::json!({}),
                    user_data: serde_json::json!({}),
                })
            } else {
                types::LoadResult::Empty {}
            }
        }
        _ => types::LoadResult::Empty {},
    })
}

/// GET /v4/info
pub async fn get_info() -> Json<types::Info> {
    Json(types::Info {
        version: types::Version {
            semver: "4.0.0".to_string(),
            major: 4,
            minor: 0,
            patch: 0,
            pre_release: None,
            build: None,
        },
        build_time: 0,
        git: types::GitInfo {
            branch: "main".to_string(),
            commit: "unknown".to_string(),
            commit_time: 0,
        },
        jvm: "Rust".to_string(),
        lavaplayer: "symphonia".to_string(),
        source_managers: vec!["http".to_string(), "youtube".to_string()],
        filters: vec![
            "volume".into(),
            "equalizer".into(),
            "karaoke".into(),
            "timescale".into(),
            "tremolo".into(),
            "vibrato".into(),
            "distortion".into(),
            "rotation".into(),
            "channelMix".into(),
            "lowPass".into(),
        ],
        plugins: vec![],
    })
}

/// GET /v4/stats
pub async fn get_stats(State(state): State<Arc<AppState>>) -> Json<types::Stats> {
    let uptime = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    Json(crate::server::collect_stats(&state, uptime))
}

/// GET /v4/version
pub async fn get_version() -> String {
    "4.0.0".to_string()
}

/// GET /v4/sessions/{sessionId}/players
pub async fn get_players(
    Path(session_id): Path<String>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    match state.sessions.get(&session_id) {
        Some(session) => {
            let players: Vec<types::Player> = session
                .players
                .iter()
                .map(|p| p.to_player_response())
                .collect();
            (StatusCode::OK, Json(serde_json::to_value(players).unwrap())).into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::to_value(types::LavalinkError::not_found(
                format!("Session not found: {}", session_id),
                format!("/v4/sessions/{}/players", session_id),
            ))
            .unwrap()),
        )
            .into_response(),
    }
}

/// GET /v4/sessions/{sessionId}/players/{guildId}
pub async fn get_player(
    Path((session_id, guild_id)): Path<(String, String)>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    match state.sessions.get(&session_id) {
        Some(session) => match session.players.get(&guild_id) {
            Some(player) => (
                StatusCode::OK,
                Json(serde_json::to_value(player.to_player_response()).unwrap()),
            )
                .into_response(),
            None => {
                // Return empty player (Lavalink behavior: player exists implicitly)
                let empty = types::Player {
                    guild_id: guild_id.clone(),
                    track: None,
                    volume: 100,
                    paused: false,
                    state: types::PlayerState {
                        time: now_ms(),
                        position: 0,
                        connected: false,
                        ping: -1,
                    },
                    voice: types::VoiceState::default(),
                    filters: types::Filters::default(),
                };
                (StatusCode::OK, Json(serde_json::to_value(empty).unwrap())).into_response()
            }
        },
        None => (
            StatusCode::NOT_FOUND,
            Json(
                serde_json::to_value(types::LavalinkError::not_found(
                    format!("Session not found: {}", session_id),
                    format!("/v4/sessions/{}/players/{}", session_id, guild_id),
                ))
                .unwrap(),
            ),
        )
            .into_response(),
    }
}

/// PATCH /v4/sessions/{sessionId}/players/{guildId}
pub async fn update_player(
    Path((session_id, guild_id)): Path<(String, String)>,
    Query(params): Query<std::collections::HashMap<String, String>>,
    State(state): State<Arc<AppState>>,
    Json(body): Json<types::PlayerUpdate>,
) -> impl IntoResponse {
    tracing::debug!(
        "Update player: session={} guild={} body={:?}",
        session_id, guild_id, body
    );

    let session = match state.sessions.get(&session_id) {
        Some(s) => s.clone(),
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(
                    serde_json::to_value(types::LavalinkError::not_found(
                        "Session not found",
                        format!("/v4/sessions/{}/players/{}", session_id, guild_id),
                    ))
                    .unwrap(),
                ),
            )
                .into_response();
        }
    };

    let no_replace = params
        .get("noReplace")
        .map(|v| v == "true")
        .unwrap_or(false);

    // Get or create player
    if !session.players.contains_key(&guild_id) {
        session
            .players
            .insert(guild_id.clone(), PlayerContext::new(guild_id.clone()));
    }

    let mut player = session.players.get_mut(&guild_id).unwrap();

    // Apply volume
    if let Some(vol) = body.volume {
        player.volume = vol;
    }

    // Apply paused
    if let Some(paused) = body.paused {
        player.paused = paused;
    }

    // Apply position (seek)
    if let Some(pos) = body.position {
        player.position = pos;
    }

    // Apply end time
    if let Some(end_time) = body.end_time {
        player.end_time = end_time;
    }

    // Apply filters (replaces entire filter chain)
    if let Some(filters) = body.filters {
        player.filters = filters;
    }

    // Apply voice
    if let Some(voice) = body.voice {
        player.voice = crate::server::VoiceConnectionState {
            token: voice.token,
            endpoint: voice.endpoint,
            session_id: voice.session_id,
            channel_id: voice.channel_id,
        };

        if let Some(uid) = session.user_id {
            let engine = player.engine.clone();
            let guild = player.guild_id.clone();
            let voice_state = player.voice.clone();
            drop(player);
            let _ = crate::server::connect_voice(engine, guild, uid, voice_state).await;
            // Re-acquire player for track handling below
            player = session.players.get_mut(&guild_id).unwrap();
        }
    }

    // Apply track
    if let Some(track_update) = body.track {
        // Check encoded first
        if let Some(encoded) = track_update.encoded {
            match encoded {
                None => {
                    // encoded: null → stop the player
                    let track_data = player.track.clone();
                    if let Some(handle) = &player.track_handle {
                        player
                            .stop_signal
                            .store(true, std::sync::atomic::Ordering::SeqCst);
                        handle.stop().await;
                    }
                    player.track_handle = None;
                    player.track = None;

                    // Emit TrackEnd with reason Stopped
                    if let Some(encoded) = track_data {
                        tracing::debug!("Emitting TrackEnd(Stopped) for guild {}", guild_id);
                        let end_event = types::OutgoingMessage::Event(
                            types::LavalinkEvent::TrackEnd {
                                guild_id: guild_id.clone(),
                                track: types::Track {
                                    encoded,
                                    info: types::TrackInfo {
                                        identifier: String::new(),
                                        is_seekable: false,
                                        author: String::new(),
                                        length: 0,
                                        is_stream: false,
                                        position: 0,
                                        title: String::new(),
                                        uri: None,
                                        artwork_url: None,
                                        isrc: None,
                                        source_name: String::new(),
                                    },
                                    plugin_info: serde_json::json!({}),
                                    user_data: serde_json::json!({}),
                                },
                                reason: types::TrackEndReason::Stopped,
                            },
                        );
                        session.send_message(&end_event).await;
                    }
                }
                Some(track_data) => {
                    // encoded: "..." → play the track
                    let is_playing = if let Some(handle) = &player.track_handle {
                        handle.get_state().await == crate::audio::playback::handle::PlaybackState::Playing
                    } else {
                        false
                    };

                    if no_replace && is_playing {
                        // noReplace=true and already playing, skip
                    } else {
                        crate::server::start_playback(&mut player, track_data, session.clone()).await;
                    }
                }
            }
        } else if let Some(identifier) = track_update.identifier {
            // identifier: "..." → resolve and play
            let is_playing = if let Some(handle) = &player.track_handle {
                handle.get_state().await == crate::audio::playback::handle::PlaybackState::Playing
            } else {
                false
            };

            if no_replace && is_playing {
                // noReplace=true and already playing, skip
            } else {
                crate::server::start_playback(&mut player, identifier, session.clone()).await;
            }
        }
    }

    let response = player.to_player_response();
    (StatusCode::OK, Json(serde_json::to_value(response).unwrap())).into_response()
}

/// DELETE /v4/sessions/{sessionId}/players/{guildId}
pub async fn destroy_player(
    Path((session_id, guild_id)): Path<(String, String)>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    tracing::debug!(
        "Destroy player: session={} guild={}",
        session_id, guild_id
    );

    match state.sessions.get(&session_id) {
        Some(session) => {
            if let Some((_, player)) = session.players.remove(&guild_id) {
                // Emit TrackEnd(Cleanup) if track existed
                if player.track.is_some() {
                    if let Some(track_data) = player.to_player_response().track {
                        tracing::debug!("Emitting TrackEnd(Cleanup) for guild {}", guild_id);
                        let end_event = types::OutgoingMessage::Event(types::LavalinkEvent::TrackEnd {
                            guild_id: guild_id.clone(),
                            track: track_data,
                            reason: types::TrackEndReason::Cleanup,
                        });
                        session.send_message(&end_event).await;
                    }
                }

                if let Some(handle) = &player.track_handle {
                    player
                        .stop_signal
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                    handle.stop().await;
                }
            }
            StatusCode::NO_CONTENT.into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(
                serde_json::to_value(types::LavalinkError::not_found(
                    "Session not found",
                    format!("/v4/sessions/{}/players/{}", session_id, guild_id),
                ))
                .unwrap(),
            ),
        )
            .into_response(),
    }
}

/// PATCH /v4/sessions/{sessionId}
pub async fn update_session(
    Path(session_id): Path<String>,
    State(state): State<Arc<AppState>>,
    Json(body): Json<types::SessionUpdate>,
) -> impl IntoResponse {
    tracing::debug!("Update session: session={} body={:?}", session_id, body);

    match state.sessions.get(&session_id) {
        Some(session) => {
            if let Some(resuming) = body.resuming {
                session
                    .resumable
                    .store(resuming, std::sync::atomic::Ordering::Relaxed);
            }
            if let Some(timeout) = body.timeout {
                session
                    .resume_timeout
                    .store(timeout, std::sync::atomic::Ordering::Relaxed);
            }

            let info = types::SessionInfo {
                resuming: session
                    .resumable
                    .load(std::sync::atomic::Ordering::Relaxed),
                timeout: session
                    .resume_timeout
                    .load(std::sync::atomic::Ordering::Relaxed),
            };

            (StatusCode::OK, Json(serde_json::to_value(info).unwrap())).into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(
                serde_json::to_value(types::LavalinkError::not_found(
                    "Session not found",
                    format!("/v4/sessions/{}", session_id),
                ))
                .unwrap(),
            ),
        )
            .into_response(),
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
