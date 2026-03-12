use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GatewayPayload {
    pub op: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<u32>,
    pub d: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum OpCode {
    Identify = 0,
    SelectProtocol = 1,
    Ready = 2,
    Heartbeat = 3,
    SessionDescription = 4,
    Speaking = 5,
    HeartbeatAck = 6,
    Resume = 7,
    Hello = 8,
    Resumed = 9,
    ClientConnect = 11,
    Video = 12,
    ClientDisconnect = 13,
    Codecs = 14,
    MediaSinkWants = 15,
    VoiceBackendVersion = 16,
    UserFlags = 18, // Undocumented but sent by Discord
    VoicePlatform = 20,
    DavePrepareTransition = 21,
    DaveExecuteTransition = 22,
    DaveTransitionReady = 23,
    DavePrepareEpoch = 24,
    MlsExternalSender = 25,
    MlsProposals = 27,
    MlsAnnounceCommitTransition = 29,
    MlsWelcome = 30,
    MlsInvalidCommitWelcome = 31,
    NoRoute = 32,
    Unknown = 255,
}

impl From<u8> for OpCode {
    /// Converts a numeric opcode into the corresponding `OpCode` variant.
    ///
    /// Unknown or unrecognized numeric values map to `OpCode::Unknown`.
    ///
    /// # Examples
    ///
    /// ```
    /// let op = OpCode::from(3u8);
    /// assert_eq!(op, OpCode::Heartbeat);
    /// ```
    fn from(op: u8) -> Self {
        match op {
            0 => Self::Identify,
            1 => Self::SelectProtocol,
            2 => Self::Ready,
            3 => Self::Heartbeat,
            4 => Self::SessionDescription,
            5 => Self::Speaking,
            6 => Self::HeartbeatAck,
            7 => Self::Resume,
            8 => Self::Hello,
            9 => Self::Resumed,
            11 => Self::ClientConnect,
            12 => Self::Video,
            13 => Self::ClientDisconnect,
            14 => Self::Codecs,
            15 => Self::MediaSinkWants,
            16 => Self::VoiceBackendVersion,
            18 => Self::UserFlags,
            20 => Self::VoicePlatform,
            21 => Self::DavePrepareTransition,
            22 => Self::DaveExecuteTransition,
            23 => Self::DaveTransitionReady,
            24 => Self::DavePrepareEpoch,
            25 => Self::MlsExternalSender,
            27 => Self::MlsProposals,
            29 => Self::MlsAnnounceCommitTransition,
            30 => Self::MlsWelcome,
            31 => Self::MlsInvalidCommitWelcome,
            32 => Self::NoRoute,
            _ => Self::Unknown,
        }
    }
}

pub mod builders {
    use serde_json::json;

    use super::*;

    /// Constructs an Identify GatewayPayload used to initiate a session.
    ///
    /// The returned payload has `op` set to the Identify opcode, `seq` omitted, and `d` containing
    /// the provided `server_id` (`guild_id`), `user_id`, `session_id`, `token`, a `video: true` flag,
    /// and `max_dave_protocol_version` set to `dave_version`.
    ///
    /// # Examples
    ///
    /// ```
    /// let payload = builders::identify(
    ///     "guild123".to_string(),
    ///     "user456".to_string(),
    ///     "session789".to_string(),
    ///     "tokenXYZ".to_string(),
    ///     2u16,
    /// );
    /// assert_eq!(payload.op, OpCode::Identify as u8);
    /// assert!(payload.seq.is_none());
    /// ```
    pub fn identify(
        guild_id: String,
        user_id: String,
        session_id: String,
        token: String,
        dave_version: u16,
    ) -> GatewayPayload {
        GatewayPayload {
            op: OpCode::Identify as u8,
            seq: None,
            d: json!({
                "server_id": guild_id,
                "user_id": user_id,
                "session_id": session_id,
                "token": token,
                "video": true,
                "max_dave_protocol_version": dave_version,
            }),
        }
    }

    /// Constructs a Resume gateway payload used to resume a previously established session.
    ///
    /// The payload's `d` field contains `server_id`, `session_id`, `token`, a `video` flag set to `true`, and the provided `seq_ack`.
    ///
    /// # Parameters
    /// - `guild_id`: ID of the server (guild) to resume.
    /// - `session_id`: Previously issued session identifier.
    /// - `token`: Authentication token for the session.
    /// - `seq_ack`: Last acknowledged sequence number to inform the gateway.
    ///
    /// # Returns
    /// A `GatewayPayload` with opcode `Resume` and the assembled JSON data in `d`.
    ///
    /// # Examples
    ///
    /// ```
    /// let payload = resume(
    ///     "guild123".to_string(),
    ///     "sess456".to_string(),
    ///     "tok789".to_string(),
    ///     42,
    /// );
    /// assert_eq!(payload.op, OpCode::Resume as u8);
    /// assert!(payload.seq.is_none());
    /// assert_eq!(payload.d["server_id"], "guild123");
    /// assert_eq!(payload.d["session_id"], "sess456");
    /// assert_eq!(payload.d["token"], "tok789");
    /// assert_eq!(payload.d["video"], true);
    /// assert_eq!(payload.d["seq_ack"], 42);
    /// ```
    pub fn resume(
        guild_id: String,
        session_id: String,
        token: String,
        seq_ack: i64,
    ) -> GatewayPayload {
        GatewayPayload {
            op: OpCode::Resume as u8,
            seq: None,
            d: json!({
                "server_id": guild_id,
                "session_id": session_id,
                "token": token,
                "video": true,
                "seq_ack": seq_ack,
            }),
        }
    }
}
