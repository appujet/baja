use std::sync::Arc;

use axum::{
  extract::{Path, Query, State},
  http::StatusCode,
  response::{IntoResponse, Json},
};

use crate::{
  api,
  api::tracks::{Track, TrackInfo},
  player::{
    Filters, Player, PlayerContext, PlayerState, PlayerUpdate, Players, VoiceConnectionState,
    VoiceState,
  },
  server::{AppState, now_ms},
};

/// GET /v4/sessions/{sessionId}/players
pub async fn get_players(
  Path(session_id): Path<String>,
  State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
  tracing::info!("GET /v4/sessions/{}/players", session_id);
  match state.sessions.get(&session_id) {
    Some(session) => {
      let players: Vec<Player> = session
        .players
        .iter()
        .map(|p| p.to_player_response())
        .collect();
      (StatusCode::OK, Json(Players { players })).into_response()
    }
    None => (
      StatusCode::NOT_FOUND,
      Json(
        serde_json::to_value(crate::common::LavalinkError::not_found(
          format!("Session not found: {}", session_id),
          format!("/v4/sessions/{}/players", session_id),
        ))
        .unwrap(),
      ),
    )
      .into_response(),
  }
}

/// GET /v4/sessions/{sessionId}/players/{guildId}
pub async fn get_player(
  Path((session_id, guild_id)): Path<(String, String)>,
  State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
  tracing::info!("GET /v4/sessions/{}/players/{}", session_id, guild_id);
  match state.sessions.get(&session_id) {
    Some(session) => match session.players.get(&guild_id) {
      Some(player) => (
        StatusCode::OK,
        Json(serde_json::to_value(player.to_player_response()).unwrap()),
      )
        .into_response(),
      None => {
        // Return empty player (Lavalink behavior: player exists implicitly)
        let empty = Player {
          guild_id: guild_id.clone(),
          track: None,
          volume: 100,
          paused: false,
          state: PlayerState {
            time: now_ms(),
            position: 0,
            connected: false,
            ping: -1,
          },
          voice: VoiceState::default(),
          filters: Filters::default(),
        };
        (StatusCode::OK, Json(serde_json::to_value(empty).unwrap())).into_response()
      }
    },
    None => (
      StatusCode::NOT_FOUND,
      Json(
        serde_json::to_value(crate::common::LavalinkError::not_found(
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
  Json(body): Json<PlayerUpdate>,
) -> impl IntoResponse {
  tracing::info!(
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
          serde_json::to_value(crate::common::LavalinkError::not_found(
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

  // Apply position (seek)
  if let Some(pos) = body.position {
    player.position = pos;
    if let Some(handle) = &player.track_handle {
      handle.seek(pos);
    }
  }

  // Apply end time
  if let Some(end_time) = body.end_time {
    player.end_time = end_time;
  }

  // Apply filters
  if let Some(filters) = body.filters {
    // Strict Validation: Check if requested filters are enabled in config
    let invalid_filters = crate::audio::filters::validate_filters(&filters, &state.config.filters);
    if !invalid_filters.is_empty() {
      let message = format!(
        "Following filters are disabled in the config: {}",
        invalid_filters.join(", ")
      );
      return (
        StatusCode::BAD_REQUEST,
        Json(
          serde_json::to_value(crate::common::LavalinkError::bad_request(
            message,
            format!("/v4/sessions/{}/players/{}", session_id, guild_id),
          ))
          .unwrap(),
        ),
      )
        .into_response();
    }

    player.filters = filters;

    // Rebuild the DSP filter chain
    let new_chain = crate::audio::filters::FilterChain::from_config(&player.filters);
    let fc = player.filter_chain.clone();
    tokio::spawn(async move {
      let mut lock = fc.lock().await;
      *lock = new_chain;
    });
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
      drop(player);
      let handle =
        crate::server::connect_voice(engine, guild, uid, voice_state, filter_chain).await;
      // Re-acquire player for track handling below
      player = session.players.get_mut(&guild_id).unwrap();

      if let Some(old_task) = player.gateway_task.replace(handle) {
        old_task.abort();
      }
    }
  }

  // 1. Resolve which track update to use (standard 'track' object or deprecated top-level fields)
  let track_to_apply = if let Some(t) = body.track {
    if body.encoded_track.is_some() || body.identifier.is_some() {
      return (
        StatusCode::BAD_REQUEST,
        Json(
          serde_json::to_value(crate::common::LavalinkError::bad_request(
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
        None => {
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
            let end_event = api::OutgoingMessage::Event(api::LavalinkEvent::TrackEnd {
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
            session.send_message(&end_event).await;
          }
        }
        Some(track_data) => {
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
              state.routeplanner.clone(),
              state.config.server.player_update_interval,
              track_update.user_data.clone(),
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
          state.routeplanner.clone(),
          state.config.server.player_update_interval,
          track_update.user_data.clone(),
        )
        .await;
      }
    }
  }

  let response = player.to_player_response();
  (
    StatusCode::OK,
    Json(serde_json::to_value(response).unwrap()),
  )
    .into_response()
}

/// DELETE /v4/sessions/{sessionId}/players/{guildId}
pub async fn destroy_player(
  Path((session_id, guild_id)): Path<(String, String)>,
  State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
  tracing::debug!("Destroy player: session={} guild={}", session_id, guild_id);

  match state.sessions.get(&session_id) {
    Some(session) => {
      if let Some((_, player)) = session.players.remove(&guild_id) {
        // Emit TrackEnd(Cleanup) if track existed
        if player.track.is_some() {
          if let Some(track_data) = player.to_player_response().track {
            tracing::debug!("Emitting TrackEnd(Cleanup) for guild {}", guild_id);
            let end_event = api::OutgoingMessage::Event(api::LavalinkEvent::TrackEnd {
              guild_id: guild_id.clone(),
              track: track_data,
              reason: api::TrackEndReason::Cleanup,
            });
            session.send_message(&end_event).await;
          }
        }

        if let Some(handle) = &player.track_handle {
          player
            .stop_signal
            .store(true, std::sync::atomic::Ordering::Relaxed);
          handle.stop();
        }
        {
          let engine = player.engine.lock().await;
          let mut mixer = engine.mixer.lock().await;
          mixer.stop_all();
        }
      }
      StatusCode::NO_CONTENT.into_response()
    }
    None => (
      StatusCode::NOT_FOUND,
      Json(
        serde_json::to_value(crate::common::LavalinkError::not_found(
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
        serde_json::to_value(crate::common::LavalinkError::not_found(
          "Session not found",
          format!("/v4/sessions/{}", session_id),
        ))
        .unwrap(),
      ),
    )
      .into_response(),
  }
}
