use crate::audio::playback::TrackHandle;
use crate::player::{Filters, Player, PlayerState, VoiceState};
use crate::track::{Track, TrackInfo};
use base64::prelude::*;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone, Default)]
pub struct VoiceConnectionState {
    pub token: String,
    pub endpoint: String,
    pub session_id: String,
    pub channel_id: Option<String>,
}

/// Internal player state.
pub struct PlayerContext {
    pub guild_id: String,
    pub volume: i32,
    pub paused: bool,
    pub track: Option<String>,
    pub track_handle: Option<TrackHandle>,
    pub position: u64,
    pub voice: VoiceConnectionState,
    pub engine: Arc<Mutex<crate::voice::VoiceEngine>>,
    pub filters: Filters,
    pub end_time: Option<u64>,
    pub stop_signal: Arc<std::sync::atomic::AtomicBool>,
    pub gateway_task: Option<tokio::task::JoinHandle<()>>,
}

impl PlayerContext {
    pub fn new(guild_id: String) -> Self {
        Self {
            guild_id,
            volume: 100,
            paused: false,
            track: None,
            track_handle: None,
            position: 0,
            voice: VoiceConnectionState::default(),
            engine: Arc::new(Mutex::new(crate::voice::VoiceEngine::new())),
            filters: Filters::default(),
            end_time: None,
            stop_signal: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            gateway_task: None,
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
            track: self.track.as_ref().map(|t| Track {
                encoded: BASE64_STANDARD.encode(t.as_bytes()),
                info: TrackInfo {
                    identifier: t.clone(),
                    is_seekable: true,
                    author: String::new(),
                    length: 0,
                    is_stream: false,
                    position: current_pos,
                    title: t.clone(),
                    uri: Some(t.clone()),
                    artwork_url: None,
                    isrc: None,
                    source_name: "http".to_string(),
                },
                plugin_info: serde_json::json!({}),
                user_data: serde_json::json!({}),
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
