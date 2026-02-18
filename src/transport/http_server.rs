use crate::server::AppState;
use crate::transport::routes::{player_routes, stats_routes};
use axum::{routing::get, Router};
use std::sync::Arc;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v4/loadtracks", get(stats_routes::load_tracks))
        .route("/v4/info", get(stats_routes::get_info))
        .route("/v4/stats", get(stats_routes::get_stats))
        .route("/version", get(stats_routes::get_version))
        .route(
            "/v4/sessions/{session_id}/players",
            get(player_routes::get_players),
        )
        .route(
            "/v4/sessions/{session_id}/players/{guild_id}",
            get(player_routes::get_player)
                .patch(player_routes::update_player)
                .delete(player_routes::destroy_player),
        )
        .route(
            "/v4/sessions/{session_id}",
            axum::routing::patch(player_routes::update_session),
        )
}
