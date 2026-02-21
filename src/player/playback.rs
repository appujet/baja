use std::sync::Arc;
use std::sync::atomic::Ordering;

use tracing::{error, info};

use super::context::PlayerContext;
use super::state::PlayerState;
use crate::{
  api,
  audio::playback::{PlaybackState, TrackHandle},
  server::Session,
};

pub async fn start_playback(
  player: &mut PlayerContext,
  track: String,
  session: Arc<Session>,
  source_manager: Arc<crate::sources::SourceManager>,
  routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
  update_interval_secs: u64,
  user_data: Option<serde_json::Value>,
) {
  if player.track.is_some() {
    let is_playing = if let Some(handle) = &player.track_handle {
      handle.get_state() != PlaybackState::Stopped
    } else {
      false
    };

    if is_playing {
      if let Some(track_data) = player.to_player_response().track {
        let end_event = api::OutgoingMessage::Event(api::LavalinkEvent::TrackEnd {
          guild_id: player.guild_id.clone(),
          track: track_data.clone(),
          reason: api::TrackEndReason::Replaced,
        });
        session.send_message(&end_event).await;
      }
    }
  }

  if let Some(task) = player.track_task.take() {
    task.abort();
  }

  if let Some(handle) = &player.track_handle {
    player.stop_signal.store(true, Ordering::SeqCst);
    handle.stop();
  }

  {
    let engine = player.engine.lock().await;
    let mut mixer = engine.mixer.lock().await;
    mixer.stop_all();
  }

  player.track = Some(track.clone());
  player.position = 0;
  player.paused = false;
  player.user_data = user_data.unwrap_or_else(|| serde_json::json!({}));
  player.stop_signal = Arc::new(std::sync::atomic::AtomicBool::new(false));

  // Decode the Lavalink track format to extract the actual playback URI
  let track_info = if let Some(decoded_track) = api::tracks::Track::decode(&track) {
    decoded_track.info
  } else {
    // If decoding fails, treat the raw string as a direct identifier
    api::tracks::TrackInfo {
      title: "Unknown".to_string(),
      author: "Unknown".to_string(),
      length: 0,
      identifier: track.clone(),
      is_stream: false,
      uri: Some(track.clone()),
      artwork_url: None,
      isrc: None,
      source_name: "unknown".to_string(),
      is_seekable: true,
      position: 0,
    }
  };

  let identifier = track_info
    .uri
    .clone()
    .unwrap_or_else(|| track_info.identifier.clone());

  let playable_track = match source_manager
    .get_track(&track_info, routeplanner.clone())
    .await
  {
    Some(t) => t,
    None => {
      error!("Failed to resolve track: {}", identifier);
      if let Some(track_data) = player.to_player_response().track {
        let event = api::OutgoingMessage::Event(api::LavalinkEvent::TrackException {
          guild_id: player.guild_id.clone(),
          track: track_data.clone(),
          exception: api::TrackException {
            message: Some(format!("Failed to resolve track: {}", identifier)),
            severity: crate::common::Severity::Common,
            cause: format!("Failed to resolve track: {}", identifier),
          },
        });
        session.send_message(&event).await;

        let end_event = api::OutgoingMessage::Event(api::LavalinkEvent::TrackEnd {
          guild_id: player.guild_id.clone(),
          track: track_data,
          reason: api::TrackEndReason::LoadFailed,
        });
        session.send_message(&end_event).await;
      }
      return;
    }
  };

  info!(
    "Playback starting: {} (source: {})",
    identifier, track_info.source_name
  );

  let (rx, cmd_tx, error_rx) = playable_track.start_decoding();
  let (handle, audio_state, vol, pos) = TrackHandle::new(cmd_tx);

  {
    let engine = player.engine.lock().await;
    let mut mixer = engine.mixer.lock().await;
    mixer.add_track(rx, audio_state, vol, pos);
  }

  player.track_handle = Some(handle.clone());

  let track_data = match player.to_player_response().track {
    Some(t) => t,
    None => {
      error!(
        "Failed to generate track response for guild {}",
        player.guild_id
      );
      return;
    }
  };
  let start_event = api::OutgoingMessage::Event(api::LavalinkEvent::TrackStart {
    guild_id: player.guild_id.clone(),
    track: track_data.clone(),
  });
  session.send_message(&start_event).await;

  let guild_id = player.guild_id.clone();
  let handle_clone = handle;
  let session_clone = session.clone();
  let stop_signal = player.stop_signal.clone();
  let track_data_clone = track_data.clone();
  let ping = player.ping.clone();

  let track_task = tokio::spawn(async move {
    let mut interval = tokio::time::interval(std::time::Duration::from_millis(500));
    let mut last_update = std::time::Instant::now();
    let update_duration = std::time::Duration::from_secs(update_interval_secs);

    let mut last_position = handle_clone.get_position();
    let mut stuck_ms = 0u64;

    loop {
      interval.tick().await;

      // Check if we should stop
      if stop_signal.load(Ordering::SeqCst) {
        break;
      }

      let current_state = handle_clone.get_state();
      if current_state == PlaybackState::Stopped {
        // Check if the decoder reported a fatal error.
        match error_rx.try_recv() {
          Ok(err_msg) => {
            tracing::warn!("[{}] Mid-playback decoder error: {}", guild_id, err_msg);
            let exception_event = api::OutgoingMessage::Event(api::LavalinkEvent::TrackException {
              guild_id: guild_id.clone(),
              track: track_data_clone.clone(),
              exception: api::TrackException {
                message: Some(err_msg.clone()),
                severity: crate::common::Severity::Fault,
                cause: err_msg,
              },
            });
            session_clone.send_message(&exception_event).await;

            let end_event = api::OutgoingMessage::Event(api::LavalinkEvent::TrackEnd {
              guild_id: guild_id.clone(),
              track: track_data_clone.clone(),
              reason: api::TrackEndReason::LoadFailed,
            });
            session_clone.send_message(&end_event).await;
          }
          Err(_) => {
            // Channel empty or disconnected — normal natural finish.
            let end_event = api::OutgoingMessage::Event(api::LavalinkEvent::TrackEnd {
              guild_id: guild_id.clone(),
              track: track_data_clone.clone(),
              reason: api::TrackEndReason::Finished,
            });
            session_clone.send_message(&end_event).await;
          }
        }
        break;
      }

      let current_pos = handle_clone.get_position();
      if current_state == PlaybackState::Playing {
        if current_pos == last_position {
          stuck_ms += 500;
          if stuck_ms == 10000 {
            let stuck_event = api::OutgoingMessage::Event(api::LavalinkEvent::TrackStuck {
              guild_id: guild_id.clone(),
              track: track_data_clone.clone(),
              threshold_ms: 10000,
            });
            session_clone.send_message(&stuck_event).await;
            tracing::warn!("Track {} got stuck!", track_data_clone.info.title);

            // Send a playerUpdate immediately after stuck — mirrors Lavalink behaviour.
            let stuck_update = api::OutgoingMessage::PlayerUpdate {
              guild_id: guild_id.clone(),
              state: PlayerState {
                time: crate::server::now_ms(),
                position: current_pos,
                connected: true,
                ping: ping.load(std::sync::atomic::Ordering::Relaxed),
              },
            };
            session_clone.send_message(&stuck_update).await;
            last_update = std::time::Instant::now();
          }
        } else {
          stuck_ms = 0;
        }
      } else {
        stuck_ms = 0;
      }
      last_position = current_pos;

      if last_update.elapsed() >= update_duration {
        last_update = std::time::Instant::now();
        let update = api::OutgoingMessage::PlayerUpdate {
          guild_id: guild_id.clone(),
          state: PlayerState {
            time: crate::server::now_ms(),
            position: current_pos,
            connected: true,
            ping: ping.load(std::sync::atomic::Ordering::Relaxed),
          },
        };
        session_clone.send_message(&update).await;
      }
    }
  });

  player.track_task = Some(track_task);
}
