use crate::server::session_manager::Session;
use dashmap::DashMap;
use std::sync::Arc;

/// Top-level application state.
pub struct AppState {
    pub sessions: DashMap<String, Arc<Session>>,
    /// Sessions disconnected but waiting for resume within timeout.
    pub resumable_sessions: DashMap<String, Arc<Session>>,
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
