use std::sync::Arc;
use tracing::{info, error};
use crate::server::Session;
use crate::player::{PlayerContext, PlayerState};
use crate::audio::playback::{PlaybackState, TrackHandle};
use crate::types;
use base64::prelude::*;

pub async fn start_playback(
    player: &mut PlayerContext,
    track: String,
    session: Arc<Session>,
) {
    if player.track.is_some() {
        let is_playing = if let Some(handle) = &player.track_handle {
            let handle: &TrackHandle = handle;
            handle.get_state().await != PlaybackState::Stopped
        } else {
            false
        };

        if is_playing {
            if let Some(track_data) = player.to_player_response().track {
                let end_event = types::OutgoingMessage::Event(types::LavalinkEvent::TrackEnd {
                    guild_id: player.guild_id.clone(),
                    track: track_data.clone(),
                    reason: types::TrackEndReason::Replaced,
                });
                session.send_message(&end_event).await;
            }
        }
    }

    if let Some(handle) = &player.track_handle {
        let handle: &TrackHandle = handle;
        player.stop_signal.store(true, std::sync::atomic::Ordering::SeqCst);
        handle.stop().await;
    }

    player.track = Some(track.clone());
    player.position = 0;
    player.paused = false;
    player.stop_signal = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let identifier = if let Ok(decoded) = BASE64_STANDARD.decode(&track) {
        String::from_utf8(decoded).unwrap_or(track.clone())
    } else {
        track.clone()
    };

    let source_manager = crate::sources::SourceManager::new();
    let playback_url = match source_manager.get_playback_url(&identifier).await {
        Some(url) => url,
        None => {
            error!("Failed to resolve URL: {}", identifier);
            return;
        }
    };

    info!("Playback: {} -> {}", identifier, playback_url);

    let rx = crate::player::decoder::start_decoding(playback_url);
    let (handle, audio_state, vol, pos) = TrackHandle::new();

    {
        let engine = player.engine.lock().await;
        let mut mixer = engine.mixer.lock().await;
        let mixer: &mut crate::audio::playback::mixer::Mixer = &mut *mixer;
        mixer.add_track(rx, audio_state, vol, pos);
    }

    player.track_handle = Some(handle);

    let track_data = player.to_player_response().track.unwrap();
    let start_event = types::OutgoingMessage::Event(types::LavalinkEvent::TrackStart {
        guild_id: player.guild_id.clone(),
        track: track_data.clone(),
    });
    session.send_message(&start_event).await;
    tracing::debug!("Sent TrackStartEvent for guild {} track {}", player.guild_id, track_data.info.title);

    let guild_id = player.guild_id.clone();
    let handle_clone = player.track_handle.as_ref().unwrap().clone();
    let session_clone = session.clone();
    let stop_signal = player.stop_signal.clone();
    let track_data_clone = track_data.clone();

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
        let mut last_update = std::time::Instant::now();

        loop {
            interval.tick().await;

            let current_state = handle_clone.get_state().await;
            if current_state == PlaybackState::Stopped {
                if !stop_signal.load(std::sync::atomic::Ordering::SeqCst) {
                    let end_event = types::OutgoingMessage::Event(types::LavalinkEvent::TrackEnd {
                        guild_id: guild_id.clone(),
                        track: track_data_clone.clone(),
                        reason: types::TrackEndReason::Finished,
                    });
                    session_clone.send_message(&end_event).await;
                }
                break;
            }

            if last_update.elapsed() >= std::time::Duration::from_secs(5) {
                last_update = std::time::Instant::now();
                let update = types::OutgoingMessage::PlayerUpdate {
                    guild_id: guild_id.clone(),
                    state: PlayerState {
                        time: crate::server::now_ms(),
                        position: handle_clone.get_position(),
                        connected: true,
                        ping: -1,
                    },
                };
                session_clone.send_message(&update).await;
            }
        }
    });
}
