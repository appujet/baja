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

/// PATCH /v4/sessions/{sessionId}/players/{guildId}
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

    let session = match state.sessions.get(&session_id) {
        Some(s) => s.clone(),
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(
                    serde_json::to_value(crate::common::RustalinkError::not_found(
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
    let player_arc = session
        .players
        .entry(guild_id.clone())
        .or_insert_with(|| {
            std::sync::Arc::new(tokio::sync::RwLock::new(PlayerContext::new(
                guild_id.clone(),
                &state.config.player,
            )))
        })
        .clone();

    let mut player = player_arc.write().await;

    // Determine if a new track will be loaded
    let loading_new_track =
        body.track.is_some() || body.encoded_track.is_some() || body.identifier.is_some();

    // 1. Apply basic state — only when NOT loading a new track
    //    (Lavalink passes these to the new track instead)
    if !loading_new_track {
        if let Some(vol) = body.volume {
            player.set_volume(vol);
        }
        if let Some(paused) = body.paused {
            player.set_paused(paused);
        }
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
    } else {
        // Volume can always be applied
        if let Some(vol) = body.volume {
            player.set_volume(vol);
        }
    }

    // 2. Apply filters
    if let Some(filters) = body.filters {
        let invalid_filters =
            crate::audio::filters::validate_filters(&filters, &state.config.filters);
        if !invalid_filters.is_empty() {
            let message = format!(
                "Following filters are disabled in the config: {}",
                invalid_filters.join(", ")
            );
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    serde_json::to_value(crate::common::RustalinkError::bad_request(
                        message,
                        format!("/v4/sessions/{}/players/{}", session_id, guild_id),
                    ))
                    .unwrap(),
                ),
            )
                .into_response();
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
    }

    // 3. Apply voice connection
    if let Some(voice) = body.voice {
        // Lavalink rejects partial voice state (empty token/endpoint/sessionId)
        if voice.token.is_empty() || voice.endpoint.is_empty() || voice.session_id.is_empty() {
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    serde_json::to_value(crate::common::RustalinkError::bad_request(
                        format!(
                            "Partial Lavalink voice state: token={} endpoint={} session_id={}",
                            voice.token, voice.endpoint, voice.session_id
                        ),
                        format!("/v4/sessions/{}/players/{}", session_id, guild_id),
                    ))
                    .unwrap(),
                ),
            )
                .into_response();
        }
        player.voice = VoiceConnectionState {
            token: voice.token,
            endpoint: voice.endpoint,
            session_id: voice.session_id,
            channel_id: voice.channel_id,
        };

        if let Some(uid) = session.user_id {
            let engine = player.engine.clone();
            let guild = player.guild_id.clone();
            let voice_state = player.voice.clone();
            let filter_chain = player.filter_chain.clone();
            let ping = player.ping.clone();
            let frames_sent = player.frames_sent.clone();
            let frames_nulled = player.frames_nulled.clone();
            drop(player);

            let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
            let session_clone = session.clone();
            tokio::spawn(async move {
                while let Some(event) = event_rx.recv().await {
                    session_clone.send_message(&protocol::OutgoingMessage::Event { event });
                }
            });

            let handle = crate::server::connect_voice(
                engine,
                guild,
                uid,
                voice_state,
                filter_chain,
                ping,
                Some(event_tx),
                frames_sent,
                frames_nulled,
            )
            .await;

            player = player_arc.write().await;
            if let Some(old_task) = player.gateway_task.replace(handle) {
                old_task.abort();
            }
        }
    }

    // 4. Resolve track update
    let no_track_change =
        body.track.is_none() && body.encoded_track.is_none() && body.identifier.is_none();

    let track_to_apply = if let Some(t) = body.track {
        if body.encoded_track.is_some() || body.identifier.is_some() {
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    serde_json::to_value(crate::common::RustalinkError::bad_request(
                        "Cannot specify both 'track' object and top-level 'encodedTrack'/'identifier'",
                        format!("/v4/sessions/{}/players/{}", session_id, guild_id),
                    ))
                    .unwrap(),
                ),
            )
                .into_response();
        }
        Some(t)
    } else if body.encoded_track.is_some() || body.identifier.is_some() {
        Some(crate::player::PlayerUpdateTrack {
            encoded: body.encoded_track,
            identifier: body.identifier,
            user_data: None,
        })
    } else {
        None
    };

    // 5. Process track update
    if let Some(track_update) = track_to_apply {
        // Lavalink: position is applied to the new track as a start offset
        let start_time_ms = if loading_new_track {
            body.position
        } else {
            None
        };
        apply_track_update(
            &mut player,
            track_update,
            session.clone(),
            &state,
            no_replace,
            body.end_time.clone(),
            start_time_ms,
        )
        .await;
    }

    // 6. Finalize end_time if no track change
    if no_track_change {
        if let Some(et) = body.end_time {
            player.end_time = match et {
                crate::player::state::EndTime::Clear => None,
                crate::player::state::EndTime::Set(val) => Some(val),
            };
        }
    }

    let response = player.to_player_response();
    (
        StatusCode::OK,
        Json(serde_json::to_value(response).unwrap()),
    )
        .into_response()
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
    let is_replacement = track_update.encoded.is_some() || track_update.identifier.is_some();
    if !is_replacement {
        if let Some(user_data) = track_update.user_data.as_ref() {
            player.user_data = user_data.clone();
        }
    }

    let end_time_val = match end_time_input {
        Some(crate::player::state::EndTime::Set(val)) => Some(val),
        _ => None,
    };

    if let Some(encoded) = track_update.encoded {
        match encoded {
            crate::player::state::TrackEncoded::Clear => {
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
                        event: protocol::RustalinkEvent::TrackEnd {
                            guild_id: player.guild_id.clone(),
                            track: protocol::tracks::Track {
                                encoded,
                                info: protocol::tracks::TrackInfo::default(),
                                plugin_info: serde_json::json!({}),
                                user_data: serde_json::json!({}),
                            },
                            reason: protocol::TrackEndReason::Stopped,
                        },
                    });
                }
            }
            crate::player::state::TrackEncoded::Set(track_data) => {
                // noReplace: Lavalink skips if track != null (regardless of paused state)
                if no_replace && player.track.is_some() {
                    return;
                }
                crate::player::start_playback(
                    player,
                    track_data,
                    session,
                    state.source_manager.clone(),
                    state.lyrics_manager.clone(),
                    state.routeplanner.clone(),
                    state.config.server.player_update_interval,
                    track_update.user_data,
                    end_time_val,
                    start_time_ms,
                )
                .await;
            }
        }
    } else if let Some(identifier) = track_update.identifier {
        // noReplace: Lavalink skips if track != null
        if no_replace && player.track.is_some() {
            return;
        }
        crate::player::start_playback(
            player,
            identifier,
            session,
            state.source_manager.clone(),
            state.lyrics_manager.clone(),
            state.routeplanner.clone(),
            state.config.server.player_update_interval,
            track_update.user_data,
            end_time_val,
            start_time_ms,
        )
        .await;
    }
}

/// PATCH /v4/sessions/{sessionId}
pub async fn update_session(
    Path(session_id): Path<crate::common::types::SessionId>,
    State(state): State<Arc<AppState>>,
    Json(body): Json<protocol::SessionUpdate>,
) -> impl IntoResponse {
    tracing::info!("PATCH /v4/sessions/{}: body={:?}", session_id, body);

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

            let info = protocol::SessionInfo {
                resuming: session.resumable.load(std::sync::atomic::Ordering::Relaxed),
                timeout: session
                    .resume_timeout
                    .load(std::sync::atomic::Ordering::Relaxed),
            };

            (StatusCode::OK, Json(serde_json::to_value(info).unwrap())).into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(
                serde_json::to_value(crate::common::RustalinkError::not_found(
                    "Session not found",
                    format!("/v4/sessions/{}", session_id),
                ))
                .unwrap(),
            ),
        )
            .into_response(),
    }
}
