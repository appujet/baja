use dashmap::DashMap;
use std::sync::Arc;

use crate::{
    common::types::SessionId, routeplanner::RoutePlanner, server::session_manager::Session,
    sources::SourceManager,
};

/// Alias for the primary session registry.
pub type SessionMap = DashMap<SessionId, Arc<Session>>;

/// Top-level application state.
pub struct AppState {
    pub sessions: SessionMap,
    /// Sessions disconnected but waiting for resume within timeout.
    pub resumable_sessions: SessionMap,
    pub routeplanner: Option<Arc<dyn RoutePlanner>>,
    pub source_manager: Arc<SourceManager>,
    pub config: crate::configs::Config,
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
