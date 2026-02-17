pub mod handlers;
pub mod models;

use crate::server::AppState;
use axum::{routing::get, Router};
use std::sync::Arc;

/// Lavalink v4 REST API router.
///
/// Endpoints:
/// - GET  /v4/loadtracks — resolve identifier to tracks
/// - GET  /v4/info — server info
/// - GET  /v4/stats — server stats
/// - GET  /v4/sessions/{sessionId}/players — list all players
/// - GET  /v4/sessions/{sessionId}/players/{guildId} — get player
/// - PATCH /v4/sessions/{sessionId}/players/{guildId} — update player
/// - DELETE /v4/sessions/{sessionId}/players/{guildId} — destroy player
/// - PATCH /v4/sessions/{sessionId} — update session (resume config)
/// - GET  /version — version string (NOT prefixed with /v4)
pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v4/loadtracks", get(handlers::load_tracks))
        .route("/v4/info", get(handlers::get_info))
        .route("/v4/stats", get(handlers::get_stats))
        .route("/version", get(handlers::get_version))
        .route(
            "/v4/sessions/{session_id}/players",
            get(handlers::get_players),
        )
        .route(
            "/v4/sessions/{session_id}/players/{guild_id}",
            get(handlers::get_player)
                .patch(handlers::update_player)
                .delete(handlers::destroy_player),
        )
        .route(
            "/v4/sessions/{session_id}",
            axum::routing::patch(handlers::update_session),
        )
}
