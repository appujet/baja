use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::common::types::AnyError;

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

/// Close codes that allow Op-7 resume (per Discord voice gateway spec).
///
/// Note: `1006` is a *local* abnormal-close marker set by the WebSocket
/// library, not a Discord gateway close code. It is handled in the WS
/// read-error arm and must NOT be listed here to avoid duplicate reconnect
/// triggers.
pub fn is_reconnectable_close(code: u16) -> bool {
    matches!(code, 4009 | 4015)
}

/// Close codes that require a fresh Identify (Op 0) instead of Resume (Op 7).
pub fn is_reidentify_close(code: u16) -> bool {
    matches!(code, 4006)
}

/// Close codes that mean the session is dead and must not be retried.
///
/// - `4004`: Authentication failed
/// - `4014`: Channel was deleted / bot was kicked
pub fn is_fatal_close(code: u16) -> bool {
    matches!(code, 4004 | 4014)
}

/// Converts any `Display`-able value into the project's boxed error type.
///
/// Using `Display` (rather than `std::error::Error`) means this works with
/// every error type in the codebase, including those that don't impl `Error`
/// (e.g. `audiopus::Error`).
#[inline]
pub fn map_boxed_err<E: std::fmt::Display>(e: E) -> AnyError {
    Box::new(std::io::Error::new(
        std::io::ErrorKind::Other,
        e.to_string(),
    ))
}
