use std::sync::{
  Arc,
  atomic::{AtomicBool, AtomicI64, Ordering},
};

use tokio::sync::Mutex;

use crate::{
  audio::{filters::FilterChain, playback::TrackHandle},
  common::types::Shared,
  player::state::{Filters, Player, PlayerState, VoiceConnectionState, VoiceState},
};

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
  pub stop_signal: Arc<AtomicBool>,
  pub ping: Arc<AtomicI64>,
  pub gateway_task: Option<tokio::task::JoinHandle<()>>,
  pub track_task: Option<tokio::task::JoinHandle<()>>,
  pub user_data: serde_json::Value,
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
      engine: Arc::new(Mutex::new(crate::gateway::VoiceEngine::new())),
      filters: Filters::default(),
      filter_chain: Arc::new(Mutex::new(FilterChain::from_config(&Filters::default()))),
      end_time: None,
      stop_signal: Arc::new(AtomicBool::new(false)),
      ping: Arc::new(AtomicI64::new(-1)),
      gateway_task: None,
      track_task: None,
      user_data: serde_json::json!({}),
    }
  }

  pub fn to_player_response(&self) -> Player {
    let track = if let Some(t) = &self.track {
      crate::api::tracks::Track::decode(t)
    } else {
      None
    };

    Player {
      guild_id: self.guild_id.clone(),
      track,
      volume: self.volume,
      paused: self.paused,
      state: PlayerState {
        time: crate::server::now_ms(),
        position: self
          .track_handle
          .as_ref()
          .map(|h| h.get_position())
          .unwrap_or(self.position),
        connected: !self.voice.token.is_empty(),
        ping: self.ping.load(Ordering::Relaxed),
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
