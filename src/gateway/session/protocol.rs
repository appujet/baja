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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_opcode_from_u8() {
        assert_eq!(OpCode::from(0), OpCode::Identify);
        assert_eq!(OpCode::from(1), OpCode::SelectProtocol);
        assert_eq!(OpCode::from(2), OpCode::Ready);
        assert_eq!(OpCode::from(3), OpCode::Heartbeat);
        assert_eq!(OpCode::from(8), OpCode::Hello);
        assert_eq!(OpCode::from(21), OpCode::DavePrepareTransition);
        assert_eq!(OpCode::from(25), OpCode::MlsExternalSender);
        assert_eq!(OpCode::from(255), OpCode::Unknown);
        assert_eq!(OpCode::from(99), OpCode::Unknown);
    }

    #[test]
    fn test_opcode_as_u8() {
        assert_eq!(OpCode::Identify as u8, 0);
        assert_eq!(OpCode::Heartbeat as u8, 3);
        assert_eq!(OpCode::Hello as u8, 8);
        assert_eq!(OpCode::DavePrepareTransition as u8, 21);
        assert_eq!(OpCode::Unknown as u8, 255);
    }

    #[test]
    fn test_opcode_equality() {
        assert_eq!(OpCode::Identify, OpCode::Identify);
        assert_ne!(OpCode::Identify, OpCode::Resume);
        assert_ne!(OpCode::Hello, OpCode::Heartbeat);
    }

    #[test]
    fn test_gateway_payload_serialization() {
        let payload = GatewayPayload {
            op: OpCode::Identify as u8,
            seq: None,
            d: serde_json::json!({"test": "value"}),
        };

        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("\"op\":0"));
        assert!(json.contains("\"test\":\"value\""));
        assert!(!json.contains("\"seq\""));
    }

    #[test]
    fn test_gateway_payload_serialization_with_seq() {
        let payload = GatewayPayload {
            op: OpCode::Heartbeat as u8,
            seq: Some(42),
            d: serde_json::json!({"t": 123456}),
        };

        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("\"op\":3"));
        assert!(json.contains("\"seq\":42"));
    }

    #[test]
    fn test_gateway_payload_deserialization() {
        let json = r#"{"op":8,"d":{"heartbeat_interval":30000}}"#;
        let payload: GatewayPayload = serde_json::from_str(json).unwrap();

        assert_eq!(payload.op, 8);
        assert_eq!(payload.seq, None);
        assert_eq!(payload.d["heartbeat_interval"], 30000);
    }

    #[test]
    fn test_builders_identify() {
        let payload = builders::identify(
            "123".to_string(),
            "456".to_string(),
            "session123".to_string(),
            "token123".to_string(),
            1,
        );

        assert_eq!(payload.op, OpCode::Identify as u8);
        assert_eq!(payload.seq, None);
        assert_eq!(payload.d["server_id"], "123");
        assert_eq!(payload.d["user_id"], "456");
        assert_eq!(payload.d["session_id"], "session123");
        assert_eq!(payload.d["token"], "token123");
        assert_eq!(payload.d["video"], true);
        assert_eq!(payload.d["max_dave_protocol_version"], 1);
    }

    #[test]
    fn test_builders_resume() {
        let payload = builders::resume(
            "123".to_string(),
            "session123".to_string(),
            "token123".to_string(),
            42,
        );

        assert_eq!(payload.op, OpCode::Resume as u8);
        assert_eq!(payload.seq, None);
        assert_eq!(payload.d["server_id"], "123");
        assert_eq!(payload.d["session_id"], "session123");
        assert_eq!(payload.d["token"], "token123");
        assert_eq!(payload.d["video"], true);
        assert_eq!(payload.d["seq_ack"], 42);
    }

    #[test]
    fn test_opcode_clone() {
        let op = OpCode::Heartbeat;
        let cloned = op.clone();
        assert_eq!(op, cloned);
    }

    #[test]
    fn test_gateway_payload_clone() {
        let payload = GatewayPayload {
            op: 5,
            seq: Some(10),
            d: serde_json::json!({"test": "data"}),
        };

        let cloned = payload.clone();
        assert_eq!(payload.op, cloned.op);
        assert_eq!(payload.seq, cloned.seq);
        assert_eq!(payload.d, cloned.d);
    }
}
