use serde::Serialize;

use crate::{api::tracks::Track, player::PlayerState};

/// Messages sent from server to client over WebSocket.
#[derive(Debug, Serialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum OutgoingMessage {
  Ready {
    resumed: bool,
    #[serde(rename = "sessionId")]
    session_id: crate::common::types::SessionId,
  },
  #[serde(rename_all = "camelCase")]
  PlayerUpdate {
    guild_id: crate::common::types::GuildId,
    state: PlayerState,
  },
  Stats(super::stats::Stats),
  Event(LavalinkEvent),
}

/// Events emitted by the player.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum LavalinkEvent {
  #[serde(rename = "TrackStartEvent")]
  #[serde(rename_all = "camelCase")]
  TrackStart {
    guild_id: crate::common::types::GuildId,
    track: Track,
  },

  #[serde(rename = "TrackEndEvent")]
  #[serde(rename_all = "camelCase")]
  TrackEnd {
    guild_id: crate::common::types::GuildId,
    track: Track,
    reason: TrackEndReason,
  },

  #[serde(rename = "TrackExceptionEvent")]
  #[serde(rename_all = "camelCase")]
  TrackException {
    guild_id: crate::common::types::GuildId,
    track: Track,
    exception: TrackException,
  },

  #[serde(rename = "TrackStuckEvent")]
  #[serde(rename_all = "camelCase")]
  TrackStuck {
    guild_id: crate::common::types::GuildId,
    track: Track,
    threshold_ms: u64,
  },

  #[serde(rename = "WebSocketClosedEvent")]
  #[serde(rename_all = "camelCase")]
  WebSocketClosed {
    guild_id: crate::common::types::GuildId,
    code: u16,
    reason: String,
    by_remote: bool,
  },
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TrackEndReason {
  Finished,
  LoadFailed,
  Stopped,
  Replaced,
  Cleanup,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackException {
  pub message: Option<String>,
  pub severity: crate::common::Severity,
  pub cause: String,
}
