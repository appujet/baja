use std::sync::{Arc, atomic::Ordering};

use tokio::time::{Duration, timeout};
use tracing::{error, info};

use super::{
    super::context::PlayerContext,
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

pub struct PlaybackStartConfig {
    pub track: String,
    pub session: Arc<Session>,
    pub source_manager: Arc<crate::sources::SourceManager>,
    pub lyrics_manager: Arc<crate::lyrics::LyricsManager>,
    pub routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    pub update_interval_secs: u64,
    pub user_data: Option<serde_json::Value>,
    pub end_time: Option<u64>,
    pub start_time_ms: Option<u64>,
}

/// Starts playback of a track on the given player according to the provided configuration.
///
/// This will stop any currently playing track, decode and resolve the requested track (with a
/// 30-second timeout), begin decoding and add the track to the audio engine mixer, send a
/// TrackStart event to the session, spawn lyric fetching, and spawn a monitor task that watches
/// playback state. If track resolution fails or building the track response fails, the function
/// returns early and no playback is started.
///
/// # Parameters
///
/// - `player`: Mutable reference to the player context whose state will be modified to start
///   playback.
/// - `config`: Configuration describing the track to start, the session, managers, start time,
///   update interval, and optional user data.
///
/// # Examples
///
/// ```no_run
/// # async fn example() {
/// // Prepare a PlayerContext and PlaybackStartConfig according to your application.
/// let mut player = /* PlayerContext */ unimplemented!();
/// let config = /* PlaybackStartConfig */ unimplemented!();
///
/// // Start playback (async)
/// start_playback(&mut player, config).await;
/// # }
/// ```
pub async fn start_playback(player: &mut PlayerContext, config: PlaybackStartConfig) {
    stop_current_track(player, &config.session).await;

    player.track_info = protocol::tracks::Track::decode(&config.track);
    player.track = Some(config.track.clone());
    player.position = 0;
    player.end_time = config.end_time;
    player.user_data = config.user_data.unwrap_or_else(|| serde_json::json!({}));
    player.stop_signal = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let track_info = player
        .track_info
        .as_ref()
        .map(|t| t.info.clone())
        .unwrap_or_else(|| protocol::tracks::TrackInfo {
            title: "Unknown".to_string(),
            author: "Unknown".to_string(),
            length: 0,
            identifier: config.track.clone(),
            is_stream: false,
            uri: Some(config.track.clone()),
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

    let playable = match timeout(
        Duration::from_secs(30),
        config
            .source_manager
            .resolve_track(&track_info, config.routeplanner),
    )
    .await
    {
        Ok(Ok(t)) => t,
        Ok(Err(e)) => {
            error!("Failed to resolve track: {} (Error: {})", identifier, e);
            send_load_failed(player, &config.session, e).await;
            return;
        }
        Err(_) => {
            error!("Track resolution timed out (30 s): {}", identifier);
            send_load_failed(
                player,
                &config.session,
                format!("Track resolution timed out: {identifier}"),
            )
            .await;
            return;
        }
    };

    info!(
        "Playback starting: {} (source: {})",
        identifier, track_info.source_name
    );

    let (frame_rx, cmd_tx, err_rx) = playable.start_decoding(player.config.clone());
    let (handle, audio_state, vol, pos) = TrackHandle::new(cmd_tx, player.tape_stop.clone());

    handle.set_volume(player.volume as f32 / 100.0);

    {
        let engine = player.engine.lock().await;
        let mut mixer = engine.mixer.lock().await;
        mixer.add_track(
            frame_rx,
            audio_state.clone(),
            vol,
            pos.clone(),
            player.config.clone(),
            48000,
        );
    }

    player.track_handle = Some(handle.clone());

    if let Some(start_ms) = config.start_time_ms
        && start_ms > 0
    {
        handle.seek(start_ms);
    }

    if player.paused {
        handle.pause();
    }

    let Some(track_response) = player.to_player_response().await.track else {
        error!(
            "Failed to build track response for guild {}",
            player.guild_id
        );
        return;
    };

    config
        .session
        .send_message(&protocol::OutgoingMessage::Event {
            event: Box::new(RustalinkEvent::TrackStart {
                guild_id: player.guild_id.clone(),
                track: track_response.clone(),
            }),
        });

    spawn_lyrics_fetch(
        player.lyrics_subscribed.clone(),
        player.lyrics_data.clone(),
        track_info.clone(),
        config.lyrics_manager,
        config.session.clone(),
        player.guild_id.clone(),
    );

    let ctx = MonitorCtx {
        guild_id: player.guild_id.clone(),
        handle: handle.clone(),
        err_rx,
        session: config.session.clone(),
        track: track_response,
        stop_signal: player.stop_signal.clone(),
        ping: player.ping.clone(),
        stuck_threshold_ms: player.config.stuck_threshold_ms,
        update_every_n: (config.update_interval_secs * 2).max(1),
        lyrics_subscribed: player.lyrics_subscribed.clone(),
        lyrics_data: player.lyrics_data.clone(),
        last_lyric_index: player.last_lyric_index.clone(),
        end_time_ms: player.end_time,
    };

    let track_task = tokio::spawn(monitor_loop(ctx));
    config.session.register_task(track_task.abort_handle());
    player.track_task = Some(track_task);
}

/// Stop playback and clean up player state.
///
/// If a track is currently playing (handle exists and is not stopped) and the player
/// has a current track, emits a `TrackEnd` event with reason `Replaced`. Then signals
/// the playback stop, aborts any running track task, stops the track handle, clears
/// playback-related fields (track, track_info, position, end_time), accumulates
/// historical frame counters onto the session, and stops all mixer channels.
///
/// # Examples
///
/// ```
/// # use tokio::runtime::Runtime;
/// # // setup_player and setup_session are hypothetical helpers for the example
/// # fn setup_player() -> PlayerContext { unimplemented!() }
/// # fn setup_session() -> Session { unimplemented!() }
/// let rt = Runtime::new().unwrap();
/// rt.block_on(async {
///     let mut player = setup_player();
///     let session = setup_session();
///     stop_current_track(&mut player, &session).await;
/// });
/// ```
async fn stop_current_track(player: &mut PlayerContext, session: &Session) {
    if let Some(handle) = &player.track_handle
        && handle.get_state() != PlaybackState::Stopped
        && let Some(track) = player.to_player_response().await.track
    {
        session.send_message(&protocol::OutgoingMessage::Event {
            event: Box::new(RustalinkEvent::TrackEnd {
                guild_id: player.guild_id.clone(),
                track,
                reason: TrackEndReason::Replaced,
            }),
        });
    }

    player.stop_signal.store(true, Ordering::Release);

    if let Some(task) = player.track_task.take() {
        task.abort();
    }

    if let Some(handle) = player.track_handle.take() {
        handle.stop();
    }
    player.track = None;
    player.track_info = None;
    player.position = 0;
    player.end_time = None;

    session.total_sent_historical.fetch_add(
        player.frames_sent.swap(0, Ordering::Relaxed),
        Ordering::Relaxed,
    );
    session.total_nulled_historical.fetch_add(
        player.frames_nulled.swap(0, Ordering::Relaxed),
        Ordering::Relaxed,
    );

    let engine = player.engine.lock().await;
    engine.mixer.lock().await.stop_all();
}
