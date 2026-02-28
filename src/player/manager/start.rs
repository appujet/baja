use std::sync::{Arc, atomic::Ordering};
use tracing::{error, info};

use super::super::context::PlayerContext;
use super::{
    error::send_load_failed,
    lyrics::spawn_lyrics_fetch,
    monitor::{MonitorCtx, monitor_loop},
};
use crate::{
    audio::playback::{PlaybackState, TrackHandle},
    protocol::{
        self,
        events::{RustalinkEvent, TrackEndReason},
    },
    server::Session,
};

/// Start playing a new track on `player`.
pub async fn start_playback(
    player: &mut PlayerContext,
    track: String,
    session: Arc<Session>,
    source_manager: Arc<crate::sources::SourceManager>,
    lyrics_manager: Arc<crate::lyrics::LyricsManager>,
    routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    update_interval_secs: u64,
    user_data: Option<serde_json::Value>,
    end_time: Option<u64>,
) {
    // -- 1. Tear down the current track ------------------------------------
    stop_current_track(player, &session).await;

    // -- 2. Set up player state for the new track --------------------------
    player.track_info = protocol::tracks::Track::decode(&track);
    player.track = Some(track.clone());
    player.position = 0;
    player.paused = false;
    player.end_time = end_time;
    player.user_data = user_data.unwrap_or_else(|| serde_json::json!({}));
    player.stop_signal = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let track_info = player
        .track_info
        .as_ref()
        .map(|t| t.info.clone())
        .unwrap_or_else(|| protocol::tracks::TrackInfo {
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
        });

    let identifier = track_info
        .uri
        .clone()
        .unwrap_or_else(|| track_info.identifier.clone());

    // -- 3. Resolve the track via SourceManager ----------------------------
    let playable = match source_manager.get_track(&track_info, routeplanner).await {
        Some(t) => t,
        None => {
            error!("Failed to resolve track: {}", identifier);
            send_load_failed(player, &session, format!("Failed to resolve: {identifier}")).await;
            return;
        }
    };

    info!(
        "Playback starting: {} (source: {})",
        identifier, track_info.source_name
    );

    // -- 4. Start decoding + hand off to mixer -----------------------------
    let (pcm_rx, cmd_tx, err_rx, opus_rx) = playable.start_decoding();
    let (handle, audio_state, vol, pos) = TrackHandle::new(cmd_tx, player.tape_stop.clone());

    {
        let engine = player.engine.lock().await;
        let mut mixer = engine.mixer.lock().await;
        mixer.add_track(
            pcm_rx,
            audio_state.clone(),
            vol,
            pos.clone(),
            player.config.clone(),
            48000,
        );
        if let Some(opus) = opus_rx {
            mixer.add_passthrough_track(opus, pos, audio_state.clone());
        }
    }

    player.track_handle = Some(handle.clone());

    if player.paused {
        handle.pause();
    }

    // -- 5. Emit TrackStart ------------------------------------------------
    let track_info_response = match player.to_player_response().track {
        Some(t) => t,
        None => {
            error!(
                "Failed to build track response for guild {}",
                player.guild_id
            );
            return;
        }
    };

    session.send_message(&protocol::OutgoingMessage::Event {
        event: RustalinkEvent::TrackStart {
            guild_id: player.guild_id.clone(),
            track: track_info_response.clone(),
        },
    });

    // -- 6. Fetch lyrics (async, non-blocking) -----------------------------
    spawn_lyrics_fetch(
        player.lyrics_subscribed.clone(),
        player.lyrics_data.clone(),
        track_info.clone(),
        lyrics_manager,
        session.clone(),
        player.guild_id.clone(),
    );

    // -- 7. Spawn 500 ms monitor loop --------------------------------------
    let ctx = MonitorCtx {
        guild_id: player.guild_id.clone(),
        handle: handle.clone(),
        err_rx,
        session: session.clone(),
        track: track_info_response.clone(),
        stop_signal: player.stop_signal.clone(),
        ping: player.ping.clone(),
        stuck_threshold_ms: player.config.stuck_threshold_ms,
        update_every_n: (update_interval_secs * 2).max(1),
        lyrics_subscribed: player.lyrics_subscribed.clone(),
        lyrics_data: player.lyrics_data.clone(),
        last_lyric_index: player.last_lyric_index.clone(),
    };

    player.track_task = Some(tokio::spawn(monitor_loop(ctx)));
}

/// Stop the currently playing track and emit `TrackEnd: Replaced` if needed.
async fn stop_current_track(player: &mut PlayerContext, session: &Session) {
    // Emit Replaced event only if something was actively playing.
    if let Some(handle) = &player.track_handle {
        if handle.get_state() != PlaybackState::Stopped {
            if let Some(track) = player.to_player_response().track {
                session.send_message(&protocol::OutgoingMessage::Event {
                    event: RustalinkEvent::TrackEnd {
                        guild_id: player.guild_id.clone(),
                        track,
                        reason: TrackEndReason::Replaced,
                    },
                });
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

    let engine = player.engine.lock().await;
    engine.mixer.lock().await.stop_all();
}
