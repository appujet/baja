use crate::api;
use crate::audio::playback::{PlaybackState, TrackHandle};
use crate::playback::{PlayerContext, PlayerState};
use crate::server::Session;
use std::sync::Arc;
use tracing::{error, info};

pub async fn start_playback(
    player: &mut PlayerContext,
    track: String,
    session: Arc<Session>,
    source_manager: Arc<crate::sources::SourceManager>,
    routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
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
        let handle: &TrackHandle = handle;
        player
            .stop_signal
            .store(true, std::sync::atomic::Ordering::SeqCst);
        handle.stop().await;
    }

    player.track = Some(track.clone());
    player.position = 0;
    player.paused = false;
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

    let playback_url = match source_manager
        .get_playback_url(&track_info, routeplanner.clone())
        .await
    {
        Some(url) => url,
        None => {
            error!("Failed to resolve URL: {}", identifier);
            return;
        }
    };

    let local_addr = if let Some(rp) = &routeplanner {
        rp.get_address()
    } else {
        None
    };

    info!(
        "Playback: {} -> {} (via {:?})",
        identifier, playback_url, local_addr
    );

    let (rx, cmd_tx) = crate::audio::pipeline::decoder::start_decoding(playback_url, local_addr);
    let (handle, audio_state, vol, pos) = TrackHandle::new(cmd_tx);

    {
        let engine = player.engine.lock().await;
        let mut mixer = engine.mixer.lock().await;
        let mixer: &mut crate::audio::playback::mixer::Mixer = &mut *mixer;
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
    tracing::debug!(
        "Sent TrackStartEvent for guild {} track {}",
        player.guild_id,
        track_data.info.title
    );

    let guild_id = player.guild_id.clone();
    let handle_clone = handle;
    let session_clone = session.clone();
    let stop_signal = player.stop_signal.clone();
    let track_data_clone = track_data.clone();

    let track_task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
        let mut last_update = std::time::Instant::now();

        loop {
            interval.tick().await;

            // Check if we should stop
            if stop_signal.load(std::sync::atomic::Ordering::SeqCst) {
                break;
            }

            let current_state = handle_clone.get_state().await;
            if current_state == PlaybackState::Stopped {
                let end_event = api::OutgoingMessage::Event(api::LavalinkEvent::TrackEnd {
                    guild_id: guild_id.clone(),
                    track: track_data_clone.clone(),
                    reason: api::TrackEndReason::Finished,
                });
                session_clone.send_message(&end_event).await;
                break;
            }

            if last_update.elapsed() >= std::time::Duration::from_secs(5) {
                last_update = std::time::Instant::now();
                let update = api::OutgoingMessage::PlayerUpdate {
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

    player.track_task = Some(track_task);
}
