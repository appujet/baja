pub mod playback;
pub mod session;
pub mod stats;
pub mod voice;

pub use playback::start_playback;
pub use session::{Session, UserId};
pub use stats::collect_stats;
pub use voice::connect_voice;

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
