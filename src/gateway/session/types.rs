use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::common::types::AnyError;

#[derive(Serialize, Deserialize, Debug)]
pub struct VoiceGatewayMessage {
    pub op: u8,
    pub d: Value,
}

/// What the outer reconnect loop should do after a session ends.
pub enum SessionOutcome {
    /// Op 7 resume is viable — the UDP/SSRC state is still valid on Discord's end.
    Reconnect,
    /// Session is stale; start fresh with Op 0 Identify.
    Identify,
    /// Close code is fatal or retry budget exhausted — stop entirely.
    Shutdown,
}

#[derive(Default)]
pub struct PersistentSessionState {
    pub ssrc: u32,
    pub udp_addr: Option<std::net::SocketAddr>,
    pub session_key: Option<[u8; 32]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CloseAction {
    UnknownOpcode,
    InvalidPayload,
    NotAuthenticated,
    AuthenticationFailed,
    AlreadyAuthenticated,
    InvalidSession,
    SessionTimeout,
    ServerNotFound,
    UnknownProtocol,
    Disconnected,
    VoiceServerCrash,
    UnknownEncryptionMode,
    InternalError,
    BadRequest,
    RateLimited,
    CallTerminated,
}

const CLOSE_CODE_TABLE: &[(u16, CloseAction)] = &[
    (4000, CloseAction::InternalError),
    (4001, CloseAction::UnknownOpcode),
    (4002, CloseAction::InvalidPayload),
    (4003, CloseAction::NotAuthenticated),
    (4004, CloseAction::AuthenticationFailed),
    (4005, CloseAction::AlreadyAuthenticated),
    (4006, CloseAction::InvalidSession),
    (4009, CloseAction::SessionTimeout),
    (4011, CloseAction::ServerNotFound),
    (4012, CloseAction::UnknownProtocol),
    (4014, CloseAction::Disconnected),
    (4015, CloseAction::VoiceServerCrash),
    (4016, CloseAction::UnknownEncryptionMode),
    (4020, CloseAction::BadRequest),
    (4021, CloseAction::RateLimited),
    (4022, CloseAction::CallTerminated),
];

/// Single authoritative entry point: given a raw WS close code, return the
/// `SessionOutcome` the reconnect loop should act on.
pub fn classify_close(code: u16) -> SessionOutcome {
    let action = CLOSE_CODE_TABLE
        .iter()
        .find(|(c, _)| *c == code)
        .map(|(_, a)| *a)
        .unwrap_or(CloseAction::UnknownOpcode);

    match action {
        CloseAction::UnknownOpcode => SessionOutcome::Reconnect,
        CloseAction::VoiceServerCrash => SessionOutcome::Reconnect,

        CloseAction::InvalidPayload => SessionOutcome::Identify,
        CloseAction::NotAuthenticated => SessionOutcome::Identify,
        CloseAction::AlreadyAuthenticated => SessionOutcome::Identify,
        CloseAction::SessionTimeout => SessionOutcome::Identify,
        CloseAction::UnknownProtocol => SessionOutcome::Identify,
        CloseAction::UnknownEncryptionMode => SessionOutcome::Identify,
        CloseAction::InternalError => SessionOutcome::Identify,
        CloseAction::BadRequest => SessionOutcome::Identify,

        // Fatal/Shutdown codes
        CloseAction::AuthenticationFailed => SessionOutcome::Shutdown,
        CloseAction::InvalidSession => SessionOutcome::Shutdown,
        CloseAction::ServerNotFound => SessionOutcome::Shutdown,
        CloseAction::Disconnected => SessionOutcome::Shutdown,
        CloseAction::RateLimited => SessionOutcome::Shutdown,
        CloseAction::CallTerminated => SessionOutcome::Shutdown,
    }
}

pub fn is_fatal_close(code: u16) -> bool {
    matches!(classify_close(code), SessionOutcome::Shutdown)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceOp {
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
    VoiceFlags = 18,
    VoicePlatform = 20,
    DavePrepareTransition = 21,
    DaveExecuteTransition = 22,
    DaveTransitionReady = 23,
    DavePrepareEpoch = 24,
    DaveMlsExternalSender = 25,
    DaveMlsKeyPackage = 26,
    DaveMlsProposals = 27,
    DaveMlsCommitWelcome = 28,
    DaveMlsAnnounceCommitTransition = 29,
    DaveMlsWelcome = 30,
    DaveMlsInvalidCommitWelcome = 31,
}

impl VoiceOp {
    pub fn from_raw(op: u8) -> Option<Self> {
        Some(match op {
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
            18 => Self::VoiceFlags,
            20 => Self::VoicePlatform,
            21 => Self::DavePrepareTransition,
            22 => Self::DaveExecuteTransition,
            23 => Self::DaveTransitionReady,
            24 => Self::DavePrepareEpoch,
            25 => Self::DaveMlsExternalSender,
            26 => Self::DaveMlsKeyPackage,
            27 => Self::DaveMlsProposals,
            28 => Self::DaveMlsCommitWelcome,
            29 => Self::DaveMlsAnnounceCommitTransition,
            30 => Self::DaveMlsWelcome,
            31 => Self::DaveMlsInvalidCommitWelcome,
            _ => return None,
        })
    }
}

#[inline]
pub fn map_boxed_err<E: std::fmt::Display>(e: E) -> AnyError {
    Box::new(std::io::Error::other(e.to_string()))
}
