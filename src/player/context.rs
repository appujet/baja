use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering},
};

use tokio::sync::Mutex;

use crate::{
    audio::{filters::FilterChain, playback::TrackHandle},
    common::types::Shared,
    config::player::PlayerConfig,
    player::state::{Filters, Player, PlayerState, VoiceConnectionState, VoiceState},
    server::AppState,
};

pub struct PlayerContext {
    pub guild_id: crate::common::types::GuildId,
    pub volume: i32,
    pub paused: bool,
    pub track: Option<String>,
    pub track_info: Option<crate::protocol::tracks::Track>,
    pub track_handle: Option<TrackHandle>,
    pub position: u64,
    pub voice: VoiceConnectionState,
    pub engine: Shared<crate::gateway::VoiceEngine>,
    pub filters: Filters,
    pub filter_chain: Shared<FilterChain>,
    pub end_time: Option<u64>,
    pub stop_signal: Arc<AtomicBool>,
    pub ping: Arc<AtomicI64>,
    pub gateway_task: Option<tokio::task::JoinHandle<()>>,
    pub track_task: Option<tokio::task::JoinHandle<()>>,
    pub user_data: serde_json::Value,
    pub frames_sent: Arc<AtomicU64>,
    pub frames_nulled: Arc<AtomicU64>,
    pub config: PlayerConfig,
    pub lyrics_subscribed: Arc<AtomicBool>,
    pub lyrics_data: Arc<Mutex<Option<crate::protocol::models::LyricsData>>>,
    pub last_lyric_index: Arc<AtomicI64>,
    pub tape_stop: Arc<AtomicBool>,
    pub state: Arc<AppState>,
}

impl PlayerContext {
    pub fn new(
        guild_id: crate::common::types::GuildId,
        config: &PlayerConfig,
        state: Arc<AppState>,
    ) -> Self {
        Self {
            guild_id,
            volume: 100,
            paused: false,
            track: None,
            track_info: None,
            track_handle: None,
            position: 0,
            voice: VoiceConnectionState::default(),
            engine: Arc::new(Mutex::new(crate::gateway::VoiceEngine::new())),
            filters: Filters::default(),
            filter_chain: Arc::new(Mutex::new(FilterChain::from_config(&Filters::default()))),
            end_time: None,
            stop_signal: Arc::new(AtomicBool::new(false)),
            ping: Arc::new(AtomicI64::new(-1)),
            gateway_task: None,
            track_task: None,
            user_data: serde_json::json!({}),
            frames_sent: Arc::new(AtomicU64::new(0)),
            frames_nulled: Arc::new(AtomicU64::new(0)),
            config: config.clone(),
            lyrics_subscribed: Arc::new(AtomicBool::new(false)),
            lyrics_data: Arc::new(Mutex::new(None)),
            last_lyric_index: Arc::new(AtomicI64::new(-1)),
            tape_stop: Arc::new(AtomicBool::new(config.tape.tape_stop)),
            state,
        }
    }

    #[inline]
    pub fn is_playing(&self) -> bool {
        self.track.is_some() && !self.paused
    }

    pub fn subscribe_lyrics(&self) {
        self.lyrics_subscribed.store(true, Ordering::Release);
        self.last_lyric_index.store(-1, Ordering::Release);
    }

    pub fn unsubscribe_lyrics(&self) {
        self.lyrics_subscribed.store(false, Ordering::Release);
    }

    pub fn set_volume(&mut self, vol: i32) {
        self.volume = vol.clamp(0, 1000);
        if let Some(handle) = &self.track_handle {
            handle.set_volume(self.volume as f32 / 100.0);
        }
    }

    pub fn set_paused(&mut self, paused: bool) {
        if self.paused == paused {
            return;
        }

        self.paused = paused;

        if let Some(handle) = &self.track_handle {
            if paused {
                handle.pause();
            } else {
                handle.play();
            }
        }
    }

    pub fn seek(&mut self, pos: u64) {
        self.position = pos;
        if let Some(handle) = &self.track_handle {
            handle.seek(pos);
        }
    }

    pub fn stop_track(&mut self) {
        self.track = None;
        self.track_info = None;
        self.position = 0;
        self.end_time = None;

        if let Some(handle) = self.track_handle.take() {
            handle.stop();
        }

        if let Some(task) = self.track_task.take() {
            task.abort();
        }

        self.stop_signal.store(true, Ordering::Release);
    }

    /// Cleanly shuts down playback and associated tasks, aborting the gateway task and stopping the audio mixer.
    ///
    /// This stops the current track, aborts an active gateway_task if present, and stops all mixers/effects in the engine.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use tokio::runtime::Runtime;
    /// # let rt = Runtime::new().unwrap();
    /// # rt.block_on(async {
    /// // given a mutable PlayerContext `ctx`
    /// ctx.destroy().await;
    /// # });
    /// ```
    pub async fn destroy(&mut self) {
        self.stop_track();

        if let Some(task) = self.gateway_task.take() {
            task.abort();
        }

        let engine = self.engine.lock().await;
        let mut mixer = engine.mixer.lock().await;
        mixer.stop_all();
    }

    /// Constructs a snapshot Player value representing the current playback and connection state.
    ///
    /// The returned Player contains the current track metadata, volume, paused flag, playback
    /// timing (timestamp and position), connection and voice details, active filters, and an
    /// optional `dave` field when the engine exposes Dave state.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use tokio::runtime::Runtime;
    /// # let rt = Runtime::new().unwrap();
    /// # rt.block_on(async {
    /// // `ctx` is a `PlayerContext` instance.
    /// let player = ctx.to_player_response().await;
    /// println!("Player for guild: {}", player.guild_id);
    /// # });
    /// ```
    pub async fn to_player_response(&self) -> Player {
        let dave = {
            let engine = self.engine.lock().await;
            if let Some(dave_shared) = &engine.dave {
                let dave = dave_shared.lock().await;
                Some(crate::player::state::DaveState {
                    protocol_version: dave.protocol_version(),
                    privacy_code: dave.voice_privacy_code(),
                })
            } else {
                None
            }
        };

        Player {
            guild_id: self.guild_id.clone(),
            track: self.track_info.clone(),
            volume: self.volume,
            paused: self.paused,
            state: PlayerState {
                time: crate::common::utils::now_ms(),
                position: self
                    .track_handle
                    .as_ref()
                    .map(|h| h.get_position())
                    .unwrap_or(self.position),
                connected: !self.voice.token.is_empty(),
                ping: self.ping.load(Ordering::Acquire),
            },
            voice: VoiceState {
                token: self.voice.token.clone(),
                endpoint: self.voice.endpoint.clone(),
                session_id: self.voice.session_id.clone(),
                channel_id: self.voice.channel_id.clone(),
            },
            filters: self.filters.clone(),
            dave,
        }
    }
}
