pub mod handlers;
pub mod models;

use crate::server::AppState;
use axum::{Router, routing::get};
use std::sync::Arc;

/// Use Case:
/// This module implements the Lavalink v4 REST API.
/// It allows external clients (like WD or Discord.js) to:
/// 1. Load tracks via /v4/loadtracks (Universal identifier resolution)
/// 2. Get node info via /v4/info
/// 3. Manage players (play, pause, volume, etc.) via /v4/sessions/{session}/players/{guild}
///
/// The track loading system uses a plugin-based architecture where each source
/// (HTTP, YouTube, Spotify, etc.) is a separate module in src/sources/
pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v4/loadtracks", get(handlers::load_tracks))
        .route("/v4/info", get(handlers::get_info))
        .route(
            "/v4/sessions/{session_id}/players/{guild_id}",
            get(handlers::get_player).patch(handlers::update_player),
        )
}
