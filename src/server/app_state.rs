use std::sync::Arc;

use dashmap::DashMap;

use crate::{routeplanner::RoutePlanner, server::session_manager::Session, sources::SourceManager};

/// Top-level application state.
pub struct AppState {
    pub sessions: DashMap<String, Arc<Session>>,
    /// Sessions disconnected but waiting for resume within timeout.
    pub resumable_sessions: DashMap<String, Arc<Session>>,
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
