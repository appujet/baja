use std::sync::Arc;

use dashmap::DashMap;

use crate::{
    common::types::SessionId, routeplanner::RoutePlanner, server::session::Session,
    sources::SourceManager,
};

/// Alias for the primary session registry.
pub type SessionMap = DashMap<SessionId, Arc<Session>>;

pub struct AppState {
    pub sessions: SessionMap,
    pub resumable_sessions: SessionMap,
    pub routeplanner: Option<Arc<dyn RoutePlanner>>,
    pub source_manager: Arc<SourceManager>,
    pub lyrics_manager: Arc<crate::lyrics::LyricsManager>,
    pub config: crate::configs::Config,
}
