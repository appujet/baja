use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Json},
};

use crate::{
    player::{PlayerContext, PlayerUpdate, VoiceConnectionState},
    protocol::{self},
    server::AppState,
};

pub async fn update_player(
    Path((session_id, guild_id)): Path<(
        crate::common::types::SessionId,
        crate::common::types::GuildId,
    )>,
    Query(params): Query<std::collections::HashMap<String, String>>,
    State(state): State<Arc<AppState>>,
    Json(body): Json<PlayerUpdate>,
) -> impl IntoResponse {
    tracing::info!(
        "PATCH /v4/sessions/{}/players/{}: body={:?}",
        session_id,
        guild_id,
        body
    );

    let Some(session) = state.sessions.get(&session_id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(crate::common::RustalinkError::not_found(
                "Session not found",
                format!("/v4/sessions/{}/players/{}", session_id, guild_id),
            )),
        )
            .into_response();
    };

    let no_replace = params
        .get("noReplace")
        .map(|v| v == "true")
        .unwrap_or(false);
    let player_arc = session.get_or_create_player(guild_id.clone(), state.clone());
    let mut player = player_arc.write().await;

    let loading_new_track =
        body.track.is_some() || body.encoded_track.is_some() || body.identifier.is_some();

    handle_player_state(&mut player, &body, loading_new_track, &guild_id, &session);

    if let Some(filters) = body.filters.clone()
        && let Err(e) = handle_filters(&mut player, filters, &state, &guild_id, &session).await
    {
        return e.into_response();
    }

    if let Some(voice) = body.voice.clone()
        && let Err(e) = handle_voice(&mut player, voice, &session, &player_arc).await
    {
        return e.into_response();
    }

    if let Some(track_update) = resolve_track_update(&body) {
        let start_time_ms = if loading_new_track {
            body.position
        } else {
            None
        };
        player.paused = body.paused.unwrap_or(false);

        apply_track_update(
            &mut player,
            track_update,
            session.clone(),
            &state,
            no_replace,
            body.end_time,
            start_time_ms,
        )
        .await;
    } else if let Some(et) = body.end_time {
        player.end_time = match et {
            crate::player::state::EndTime::Clear => None,
            crate::player::state::EndTime::Set(val) => Some(val),
        };
    }

    let response = player.to_player_response();
    (StatusCode::OK, Json(response)).into_response()
}

fn handle_player_state(
    player: &mut PlayerContext,
    body: &PlayerUpdate,
    loading_new_track: bool,
    guild_id: &crate::common::types::GuildId,
    session: &Arc<crate::server::Session>,
) {
    if !loading_new_track {
        if let Some(pos) = body.position {
            player.seek(pos);
            if player.track.is_some() {
                let seek_update = protocol::OutgoingMessage::PlayerUpdate {
                    guild_id: guild_id.clone(),
                    state: crate::player::PlayerState {
                        time: crate::common::utils::now_ms(),
                        position: pos,
                        connected: !player.voice.token.is_empty(),
                        ping: player.ping.load(std::sync::atomic::Ordering::Relaxed),
                    },
                };
                let session_clone = session.clone();
                tokio::spawn(async move {
                    session_clone.send_message(&seek_update);
                });
            }
        }
        if let Some(paused) = body.paused {
            player.set_paused(paused);
        }
    }

    if let Some(vol) = body.volume {
        player.set_volume(vol);
    }
}

async fn handle_filters(
    player: &mut PlayerContext,
    filters: crate::player::Filters,
    state: &AppState,
    guild_id: &crate::common::types::GuildId,
    session: &Arc<crate::server::Session>,
) -> Result<(), (StatusCode, Json<crate::common::RustalinkError>)> {
    let invalid_filters = crate::audio::filters::validate_filters(&filters, &state.config.filters);
    if !invalid_filters.is_empty() {
        let message = format!(
            "Following filters are disabled in the config: {}",
            invalid_filters.join(", ")
        );
        return Err((
            StatusCode::BAD_REQUEST,
            Json(crate::common::RustalinkError::bad_request(
                message,
                format!("/v4/sessions/xxx/players/{}", guild_id), // session content isn't strictly needed for response body here
            )),
        ));
    }

    player.filters = filters;
    let new_chain = crate::audio::filters::FilterChain::from_config(&player.filters);
    {
        let mut lock = player.filter_chain.lock().await;
        *lock = new_chain;
    }

    session.send_message(&protocol::OutgoingMessage::PlayerUpdate {
        guild_id: guild_id.clone(),
        state: crate::player::PlayerState {
            time: crate::common::utils::now_ms(),
            position: player
                .track_handle
                .as_ref()
                .map(|h| h.get_position())
                .unwrap_or(player.position),
            connected: !player.voice.token.is_empty(),
            ping: player.ping.load(std::sync::atomic::Ordering::Relaxed),
        },
    });

    Ok(())
}

async fn handle_voice(
    player: &mut PlayerContext,
    voice: crate::player::VoiceState,
    session: &Arc<crate::server::Session>,
    _player_arc: &Arc<tokio::sync::RwLock<PlayerContext>>,
) -> Result<(), (StatusCode, Json<crate::common::RustalinkError>)> {
    if voice.token.is_empty() || voice.endpoint.is_empty() || voice.session_id.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(crate::common::RustalinkError::bad_request(
                "Partial Lavalink voice state",
                "/v4/voice_state",
            )),
        ));
    }

    player.voice = VoiceConnectionState {
        token: voice.token,
        endpoint: voice.endpoint,
        session_id: voice.session_id,
        channel_id: voice.channel_id,
    };

    if let Some(uid) = session.user_id {
        let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
        let session_clone = session.clone();
        tokio::spawn(async move {
            while let Some(event) = event_rx.recv().await {
                session_clone.send_message(&protocol::OutgoingMessage::Event {
                    event: Box::new(event),
                });
            }
        });

        let handle = crate::server::connect_voice(crate::server::voice::VoiceConnectConfig {
            engine: player.engine.clone(),
            guild_id: player.guild_id.clone(),
            user_id: uid,
            voice: player.voice.clone(),
            filter_chain: player.filter_chain.clone(),
            ping: player.ping.clone(),
            event_tx: Some(event_tx),
            frames_sent: player.frames_sent.clone(),
            frames_nulled: player.frames_nulled.clone(),
        })
        .await;

        session.register_task(handle.abort_handle());
        if let Some(old_task) = player.gateway_task.replace(handle) {
            old_task.abort();
        }
    }

    Ok(())
}

fn resolve_track_update(body: &PlayerUpdate) -> Option<crate::player::PlayerUpdateTrack> {
    if let Some(t) = &body.track {
        Some(t.clone())
    } else if let Some(et) = &body.encoded_track {
        Some(crate::player::PlayerUpdateTrack {
            encoded: Some(et.clone()),
            identifier: body.identifier.clone(),
            user_data: None,
        })
    } else {
        body.identifier
            .as_ref()
            .map(|ident| crate::player::PlayerUpdateTrack {
                encoded: None,
                identifier: Some(ident.clone()),
                user_data: None,
            })
    }
}

async fn apply_track_update(
    player: &mut PlayerContext,
    track_update: crate::player::PlayerUpdateTrack,
    session: Arc<crate::server::Session>,
    state: &AppState,
    no_replace: bool,
    end_time_input: Option<crate::player::state::EndTime>,
    start_time_ms: Option<u64>,
) {
    if let Some(track_data) = track_update.encoded {
        match track_data {
            crate::player::state::TrackEncoded::Clear => {
                stop_player(player, &session).await;
            }
            crate::player::state::TrackEncoded::Set(encoded) => {
                if no_replace && player.track.is_some() {
                    return;
                }
                start_playback(
                    player,
                    encoded,
                    track_update.user_data,
                    session,
                    state,
                    end_time_input,
                    start_time_ms,
                )
                .await;
            }
        }
    } else if let Some(identifier) = track_update.identifier {
        if no_replace && player.track.is_some() {
            return;
        }
        start_playback(
            player,
            identifier,
            track_update.user_data,
            session,
            state,
            end_time_input,
            start_time_ms,
        )
        .await;
    } else if let Some(user_data) = track_update.user_data {
        player.user_data = user_data;
    }
}

async fn stop_player(player: &mut PlayerContext, session: &Arc<crate::server::Session>) {
    let track_data = player.track.clone();
    if let Some(handle) = &player.track_handle {
        player
            .stop_signal
            .store(true, std::sync::atomic::Ordering::SeqCst);
        handle.stop();
    }
    {
        let engine = player.engine.lock().await;
        let mut mixer = engine.mixer.lock().await;
        mixer.stop_all();
    }
    player.track_handle = None;
    player.track = None;

    if let Some(encoded) = track_data {
        session.send_message(&protocol::OutgoingMessage::Event {
            event: Box::new(protocol::RustalinkEvent::TrackEnd {
                guild_id: player.guild_id.clone(),
                track: protocol::tracks::Track {
                    encoded,
                    info: protocol::tracks::TrackInfo::default(),
                    plugin_info: serde_json::json!({}),
                    user_data: serde_json::json!({}),
                },
                reason: protocol::TrackEndReason::Stopped,
            }),
        });
    }
}

async fn start_playback(
    player: &mut PlayerContext,
    track: String,
    user_data: Option<serde_json::Value>,
    session: Arc<crate::server::Session>,
    state: &AppState,
    end_time_input: Option<crate::player::state::EndTime>,
    start_time_ms: Option<u64>,
) {
    let end_time = match end_time_input {
        Some(crate::player::state::EndTime::Set(val)) => Some(val),
        _ => None,
    };

    crate::player::start_playback(
        player,
        crate::player::manager::start::PlaybackStartConfig {
            track,
            session,
            source_manager: state.source_manager.clone(),
            lyrics_manager: state.lyrics_manager.clone(),
            routeplanner: state.routeplanner.clone(),
            update_interval_secs: state.config.server.player_update_interval,
            user_data,
            end_time,
            start_time_ms,
        },
    )
    .await;
}

/// PATCH /v4/sessions/{sessionId}
pub async fn update_session(
    Path(session_id): Path<crate::common::types::SessionId>,
    State(state): State<Arc<AppState>>,
    Json(body): Json<protocol::SessionUpdate>,
) -> impl IntoResponse {
    tracing::info!("PATCH /v4/sessions/{}: body={:?}", session_id, body);

    let Some(session) = state.sessions.get(&session_id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(crate::common::RustalinkError::not_found(
                "Session not found",
                format!("/v4/sessions/{}", session_id),
            )),
        )
            .into_response();
    };

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

    let info = protocol::SessionInfo {
        resuming: session.resumable.load(std::sync::atomic::Ordering::Relaxed),
        timeout: session
            .resume_timeout
            .load(std::sync::atomic::Ordering::Relaxed),
    };

    (StatusCode::OK, Json(info)).into_response()
}
