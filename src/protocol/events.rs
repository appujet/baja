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

/// Events emitted by the player (op = "event").
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
        /// `true` if Discord closed the connection; `false` if Lavalink/Rustalink did.
        #[serde(rename = "byRemote")]
        by_remote: bool,
    },
}

/// Why a track stopped playing.
///
/// Serialized as lowercase/camelCase to match the Lavalink v4 wire format exactly.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TrackEndReason {
    /// Track played to the end (or ended due to an exception after starting).
    /// `mayStartNext = true`
    #[serde(rename = "finished")]
    Finished,

    /// Track failed to start before providing any audio.
    /// `mayStartNext = true`
    #[serde(rename = "loadFailed")]
    LoadFailed,

    /// Player was explicitly stopped via stop() or play(null).
    /// `mayStartNext = false`
    #[serde(rename = "stopped")]
    Stopped,

    /// A new track started playing, replacing this one.
    /// `mayStartNext = false`
    #[serde(rename = "replaced")]
    Replaced,

    /// Player cleanup threshold reached (leaked/idle player).
    /// `mayStartNext = false`
    #[serde(rename = "cleanup")]
    Cleanup,
}

/// Exception details for `TrackExceptionEvent`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackException {
    pub message: Option<String>,
    pub severity: crate::common::Severity,
    pub cause: String,
    pub cause_stack_trace: Option<String>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_track_end_reason_serialization() {
        assert_eq!(
            serde_json::to_string(&TrackEndReason::Finished).unwrap(),
            "\"finished\""
        );
        assert_eq!(
            serde_json::to_string(&TrackEndReason::LoadFailed).unwrap(),
            "\"loadFailed\""
        );
        assert_eq!(
            serde_json::to_string(&TrackEndReason::Stopped).unwrap(),
            "\"stopped\""
        );
        assert_eq!(
            serde_json::to_string(&TrackEndReason::Replaced).unwrap(),
            "\"replaced\""
        );
        assert_eq!(
            serde_json::to_string(&TrackEndReason::Cleanup).unwrap(),
            "\"cleanup\""
        );
    }

    #[test]
    fn test_track_end_reason_deserialization() {
        let r: TrackEndReason = serde_json::from_str("\"finished\"").unwrap();
        assert!(matches!(r, TrackEndReason::Finished));

        let r: TrackEndReason = serde_json::from_str("\"loadFailed\"").unwrap();
        assert!(matches!(r, TrackEndReason::LoadFailed));

        let r: TrackEndReason = serde_json::from_str("\"stopped\"").unwrap();
        assert!(matches!(r, TrackEndReason::Stopped));
    }
}
