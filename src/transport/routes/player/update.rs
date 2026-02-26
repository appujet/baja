use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Json},
};

use crate::{
    api,
    api::tracks::{Track, TrackInfo},
    player::{PlayerContext, PlayerUpdate, VoiceConnectionState},
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
    tracing::debug!(
        "PATCH /v4/sessions/{}/players/{}\n{:?}",
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
    if !session.players.contains_key(&guild_id) {
        session.players.insert(
            guild_id.clone(),
            PlayerContext::new(guild_id.clone(), &state.config.player),
        );
    }

    let mut player = session.players.get_mut(&guild_id).unwrap();

    // Apply volume
    if let Some(vol) = body.volume {
        let vol = vol.clamp(0, 1000);
        player.volume = vol;

        // Propagate to active track handle for immediate effect
        if let Some(handle) = &player.track_handle {
            handle.set_volume(vol as f32 / 100.0);
        }
    }

    // Apply paused
    if let Some(paused) = body.paused {
        player.paused = paused;
        if let Some(handle) = &player.track_handle {
            if paused {
                handle.pause();
            } else {
                handle.play();
            }
        }
    }

    // Apply position (seek) — also send playerUpdate
    if let Some(pos) = body.position {
        player.position = pos;
        if player.track.is_some() {
            if let Some(handle) = &player.track_handle {
                handle.seek(pos);
            }
            // Send playerUpdate immediately after seek
            let seek_update = api::OutgoingMessage::PlayerUpdate {
                guild_id: guild_id.clone(),
                state: crate::player::PlayerState {
                    time: crate::server::now_ms(),
                    position: pos,
                    connected: !player.voice.token.is_empty(),
                    ping: player.ping.load(std::sync::atomic::Ordering::Relaxed),
                },
            };
            let session_seek = session.clone();
            tokio::spawn(async move {
                session_seek.send_message(&seek_update);
            });
        }
    }

    // end_time is passed into start_playback below
    let end_time_val = match body.end_time {
        Some(crate::player::state::EndTime::Set(val)) => Some(val),
        _ => None,
    };

    // Apply filters
    if let Some(filters) = body.filters {
        // Strict Validation: Check if requested filters are enabled in config
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
    }

    // Apply voice
    if let Some(voice) = body.voice {
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
                    let msg = api::OutgoingMessage::Event(event);
                    session_clone.send_message(&msg);
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
            // Re-acquire player for track handling below
            player = session.players.get_mut(&guild_id).unwrap();

            if let Some(old_task) = player.gateway_task.replace(handle) {
                old_task.abort();
            }
        }
    }

    // 1. Resolve which track update to use (standard 'track' object or deprecated top-level fields)
    // Capture this before body fields are moved.
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

    // 2. Process track update if present
    if let Some(track_update) = track_to_apply {
        let is_replacement = track_update.encoded.is_some() || track_update.identifier.is_some();
        if !is_replacement {
            if let Some(user_data) = track_update.user_data.as_ref() {
                player.user_data = user_data.clone();
            }
        }

        if let Some(encoded) = track_update.encoded {
            match encoded {
                crate::player::state::TrackEncoded::Clear => {
                    // encoded: null → stop the player
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

                    // Emit TrackEnd with reason Stopped
                    if let Some(encoded) = track_data {
                        let end_event =
                            api::OutgoingMessage::Event(api::RustalinkEvent::TrackEnd {
                                guild_id: guild_id.clone(),
                                track: Track {
                                    encoded,
                                    info: TrackInfo {
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
                                reason: api::TrackEndReason::Stopped,
                            });
                        session.send_message(&end_event);
                    }
                }
                crate::player::state::TrackEncoded::Set(track_data) => {
                    let is_playing = if let Some(handle) = &player.track_handle {
                        handle.get_state() == crate::audio::playback::handle::PlaybackState::Playing
                    } else {
                        false
                    };

                    if !no_replace || !is_playing {
                        crate::player::start_playback(
                            &mut player,
                            track_data,
                            session.clone(),
                            state.source_manager.clone(),
                            state.lyrics_manager.clone(),
                            state.routeplanner.clone(),
                            state.config.server.player_update_interval,
                            track_update.user_data.clone(),
                            end_time_val,
                        )
                        .await;
                    }
                }
            }
        } else if let Some(identifier) = track_update.identifier {
            let is_playing = if let Some(handle) = &player.track_handle {
                handle.get_state() == crate::audio::playback::handle::PlaybackState::Playing
            } else {
                false
            };

            if !no_replace || !is_playing {
                crate::player::start_playback(
                    &mut player,
                    identifier,
                    session.clone(),
                    state.source_manager.clone(),
                    state.lyrics_manager.clone(),
                    state.routeplanner.clone(),
                    state.config.server.player_update_interval,
                    track_update.user_data.clone(),
                    end_time_val,
                )
                .await;
            }
        }
    }

    // If no track was changed but endTime was provided (even null), update it on the active player
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

/// PATCH /v4/sessions/{sessionId}
pub async fn update_session(
    Path(session_id): Path<crate::common::types::SessionId>,
    State(state): State<Arc<AppState>>,
    Json(body): Json<api::SessionUpdate>,
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

            let info = api::SessionInfo {
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
