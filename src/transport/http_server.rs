use std::sync::Arc;

use axum::{
    Router,
    extract::{Request, State},
    http::{HeaderValue, StatusCode},
    middleware::{self, Next},
    response::Response,
    routing::get,
};
use tracing::warn;

use crate::{
    server::AppState,
    transport::routes::{player_routes, stats_routes},
};

async fn check_auth(
    State(state): State<Arc<AppState>>,
    req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    // Clone the header value to separate it from the request lifetime.
    // We cannot borrow `req` (via headers) and then move `req` into `next.run(req)`.
    let auth_header = req
        .headers()
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .map(String::from);

    match auth_header {
        Some(auth) if auth == state.config.server.password => Ok(next.run(req).await),
        Some(_) => {
            warn!("REST Authorization failed: Invalid password");
            Err(StatusCode::UNAUTHORIZED)
        }
        None => {
            warn!("REST Authorization failed: Missing Authorization header");
            Err(StatusCode::UNAUTHORIZED)
        }
    }
}

async fn add_response_headers(req: Request, next: Next) -> Response {
    let mut response = next.run(req).await;
    // Essential for Lavalink clients to verify v4 compatibility
    response
        .headers_mut()
        .insert("Lavalink-Api-Version", HeaderValue::from_static("4"));
    response
}

pub fn router(state: Arc<AppState>) -> Router {
    let api_routes = Router::new()
        .route("/v4/loadtracks", get(stats_routes::load_tracks))
        .route("/v4/info", get(stats_routes::get_info))
        .route("/v4/stats", get(stats_routes::get_stats))
        .route("/version", get(stats_routes::get_version))
        .route("/v4/decodetrack", get(stats_routes::decode_track))
        .route(
            "/v4/decodetracks",
            axum::routing::post(stats_routes::decode_tracks),
        )
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
        // Routeplanner
        .route(
            "/v4/routeplanner/status",
            get(stats_routes::routeplanner_status),
        )
        .route(
            "/v4/routeplanner/free/address",
            axum::routing::post(stats_routes::routeplanner_free_address),
        )
        .route(
            "/v4/routeplanner/free/all",
            axum::routing::post(stats_routes::routeplanner_free_all),
        )
        // Inject state so middleware can extract it
        .layer(middleware::from_fn_with_state(state.clone(), check_auth))
        // Convert back to Router<()> for merging
        .with_state(state);

    Router::new()
        .merge(api_routes)
        .layer(middleware::from_fn(add_response_headers))
}
