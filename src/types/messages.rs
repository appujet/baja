use super::player::PlayerState;
use super::track::Track;
use serde::Serialize;

/// Messages sent from server to client over WebSocket.
#[derive(Debug, Serialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum OutgoingMessage {
    Ready {
        resumed: bool,
        #[serde(rename = "sessionId")]
        session_id: String,
    },
    #[serde(rename_all = "camelCase")]
    PlayerUpdate {
        guild_id: String,
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
    TrackStart { guild_id: String, track: Track },
    #[serde(rename = "TrackEndEvent")]
    #[serde(rename_all = "camelCase")]
    TrackEnd {
        guild_id: String,
        track: Track,
        reason: TrackEndReason,
    },
    #[serde(rename = "TrackExceptionEvent")]
    #[serde(rename_all = "camelCase")]
    #[allow(dead_code)]
    TrackException {
        guild_id: String,
        track: Track,
        exception: TrackException,
    },
    #[serde(rename = "TrackStuckEvent")]
    #[serde(rename_all = "camelCase")]
    #[allow(dead_code)]
    TrackStuck {
        guild_id: String,
        track: Track,
        threshold_ms: u64,
    },
    #[serde(rename = "WebSocketClosedEvent")]
    #[serde(rename_all = "camelCase")]
    #[allow(dead_code)]
    WebSocketClosed {
        guild_id: String,
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
#[allow(dead_code)]
pub struct TrackException {
    pub message: Option<String>,
    pub severity: super::error::Severity,
    pub cause: String,
}
