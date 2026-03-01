use serde::Serialize;

use crate::{player::PlayerState, protocol::tracks::Track};

/// Messages sent from server to client over WebSocket.
#[derive(Debug, Serialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum OutgoingMessage {
    Ready {
        resumed: bool,
        #[serde(rename = "sessionId")]
        session_id: crate::common::types::SessionId,
    },
    #[serde(rename = "playerUpdate")]
    PlayerUpdate {
        guild_id: crate::common::types::GuildId,
        state: PlayerState,
    },
    #[serde(rename = "stats")]
    Stats {
        #[serde(flatten)]
        stats: super::stats::Stats,
    },
    #[serde(rename = "event")]
    Event {
        #[serde(flatten)]
        event: RustalinkEvent,
    },
}

/// Events emitted by the player.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum RustalinkEvent {
    #[serde(rename = "TrackStartEvent")]
    TrackStart {
        #[serde(rename = "guildId")]
        guild_id: crate::common::types::GuildId,
        track: Track,
    },

    #[serde(rename = "TrackEndEvent")]
    TrackEnd {
        #[serde(rename = "guildId")]
        guild_id: crate::common::types::GuildId,
        track: Track,
        reason: TrackEndReason,
    },

    #[serde(rename = "TrackExceptionEvent")]
    TrackException {
        #[serde(rename = "guildId")]
        guild_id: crate::common::types::GuildId,
        track: Track,
        exception: TrackException,
    },

    #[serde(rename = "TrackStuckEvent")]
    TrackStuck {
        #[serde(rename = "guildId")]
        guild_id: crate::common::types::GuildId,
        track: Track,
        #[serde(rename = "thresholdMs")]
        threshold_ms: u64,
    },

    #[serde(rename = "LyricsFoundEvent")]
    LyricsFound {
        #[serde(rename = "guildId")]
        guild_id: crate::common::types::GuildId,
        lyrics: super::models::RustalinkLyrics,
    },

    #[serde(rename = "LyricsNotFoundEvent")]
    LyricsNotFound {
        #[serde(rename = "guildId")]
        guild_id: crate::common::types::GuildId,
    },

    #[serde(rename = "LyricsLineEvent")]
    LyricsLine {
        #[serde(rename = "guildId")]
        guild_id: crate::common::types::GuildId,
        line_index: i32,
        line: super::models::RustalinkLyricsLine,
        skipped: bool,
    },

    #[serde(rename = "WebSocketClosedEvent")]
    WebSocketClosed {
        #[serde(rename = "guildId")]
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
    pub cause_stack_trace: Option<String>,
}
