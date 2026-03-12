use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum GatewayError {
    #[error("WebSocket error: {0}")]
    WebSocket(#[from] tokio_tungstenite::tungstenite::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Discovery failed: {0}")]
    Discovery(String),

    #[error("Encoding error: {0}")]
    Encoding(String),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Other error: {0}")]
    Other(#[from] Box<dyn std::error::Error + Send + Sync>),
}

/// Converts a displayable error into the crate's boxed `AnyError`.
///
/// The provided value's `Display` output is used to construct a `std::io::Error` via
/// `std::io::Error::other`, which is then boxed as `crate::common::types::AnyError`.
///
/// # Examples
///
/// ```
/// let any_err = map_boxed_err("connection failed");
/// assert_eq!(any_err.to_string(), "connection failed");
/// ```
pub fn map_boxed_err<E: std::fmt::Display>(e: E) -> crate::common::types::AnyError {
    Box::new(std::io::Error::other(e.to_string()))
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionOutcome {
    Reconnect,
    Identify,
    Shutdown,
}

#[derive(Default, Debug)]
pub struct PersistentSessionState {
    pub ssrc: u32,
    pub udp_addr: Option<std::net::SocketAddr>,
    pub session_key: Option<[u8; 32]>,
    pub rtp_state: Option<crate::gateway::udp_link::RtpState>,
    pub selected_mode: Option<String>,
}
