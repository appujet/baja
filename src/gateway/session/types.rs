use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Serialize, Deserialize, Debug)]
pub struct VoiceGatewayMessage {
    pub op: u8,
    pub d: Value,
}

/// Outcome of a single WS session — tells the outer loop what to do next.
pub enum SessionOutcome {
    /// Reconnectable disconnect — try Op 7 resume.
    Reconnect,
    /// Session invalid — start over with fresh Op 0 Identify.
    Identify,
    /// Fatal close or max errors — stop entirely.
    Shutdown,
}

/// Close codes that allow reconnection (per Discord voice gateway spec).
pub fn is_reconnectable_close(code: u16) -> bool {
    matches!(code, 1006 | 4015 | 4009)
}

/// Close codes that require a fresh Identify (Op 0) instead of Resume (Op 7).
pub fn is_reidentify_close(code: u16) -> bool {
    matches!(code, 4006)
}

/// Close codes that mean the session is dead and shouldn't be retried.
pub fn is_fatal_close(code: u16) -> bool {
    // 4004: Authentication failed
    // 4014: Channel was deleted / bot was kicked
    // 4022: Call terminated (disconnected by user/bot leave)
    matches!(code, 4004 | 4014 | 4022)
}

pub fn map_boxed_err<E: std::fmt::Display>(e: E) -> crate::common::types::AnyError {
    Box::new(std::io::Error::new(
        std::io::ErrorKind::Other,
        e.to_string(),
    ))
}
