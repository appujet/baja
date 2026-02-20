
use crate::common::types::{Shared};
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::{
    api::tracks::Track,
    audio::{filters::FilterChain, playback::TrackHandle},
    playback::{Filters, Player, PlayerState, VoiceConnectionState, VoiceState},
};

/// Internal player state.
pub struct PlayerContext {
    pub guild_id: String,
    pub volume: i32,
    pub paused: bool,
    pub track: Option<String>,
    pub track_handle: Option<TrackHandle>,
    pub position: u64,
    pub voice: VoiceConnectionState,
    pub engine: Shared<crate::gateway::VoiceEngine>,
    pub filters: Filters,
    pub filter_chain: Shared<FilterChain>,
    pub end_time: Option<u64>,
    pub stop_signal: Arc<std::sync::atomic::AtomicBool>,
    pub gateway_task: Option<tokio::task::JoinHandle<()>>,
    pub track_task: Option<tokio::task::JoinHandle<()>>,
}

impl PlayerContext {
    pub fn new(guild_id: String) -> Self {
        let filters = Filters::default();
        let filter_chain = Arc::new(Mutex::new(FilterChain::from_config(&filters)));
        Self {
            guild_id,
            volume: 100,
            paused: false,
            track: None,
            track_handle: None,
            position: 0,
            voice: VoiceConnectionState::default(),
            engine: Arc::new(Mutex::new(crate::gateway::VoiceEngine::new())),
            filters,
            filter_chain,
            end_time: None,
            stop_signal: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            gateway_task: None,
            track_task: None,
        }
    }

    pub fn to_player_response(&self) -> Player {
        let current_pos = self
            .track_handle
            .as_ref()
            .map(|h| h.get_position())
            .unwrap_or(self.position);

        Player {
            guild_id: self.guild_id.clone(),
            track: self.track.as_ref().and_then(|t| {
                let mut track = Track::decode(t);
                if let Some(ref mut trk) = track {
                    trk.info.position = current_pos;
                }
                track
            }),
            volume: self.volume,
            paused: self.paused,
            state: PlayerState {
                time: crate::server::now_ms(),
                position: current_pos,
                connected: !self.voice.token.is_empty(),
                ping: -1,
            },
            voice: VoiceState {
                token: self.voice.token.clone(),
                endpoint: self.voice.endpoint.clone(),
                session_id: self.voice.session_id.clone(),
                channel_id: self.voice.channel_id.clone(),
            },
            filters: self.filters.clone(),
        }
    }
}

impl Drop for PlayerContext {
    fn drop(&mut self) {
        if let Some(task) = &self.gateway_task {
            tracing::debug!("Aborting gateway task for guild {}", self.guild_id);
            task.abort();
        }
        if let Some(task) = &self.track_task {
            tracing::debug!("Aborting track task for guild {}", self.guild_id);
            task.abort();
        }
    }
}
