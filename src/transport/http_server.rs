use std::sync::Arc;

use axum::{
  Router,
  extract::{Request, State},
  http::{HeaderValue, StatusCode},
  middleware::{self, Next},
  response::Response,
  routing::{get, post, patch},
};
use tracing::warn;

use crate::{
  server::AppState,
  transport::routes::{player, stats},
};

async fn check_auth(
  State(state): State<Arc<AppState>>,
  req: Request,
  next: Next,
) -> Result<Response, StatusCode> {
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
  response
    .headers_mut()
    .insert("Lavalink-Api-Version", HeaderValue::from_static("4"));
  response
}

pub fn router(state: Arc<AppState>) -> Router {
  let api_routes = Router::new()
    .route("/v4/loadtracks", get(stats::load_tracks))
    .route("/v4/info", get(stats::get_info))
    .route("/v4/stats", get(stats::get_stats))
    .route("/version", get(stats::get_version))
    .route("/v4/decodetrack", get(stats::decode_track))
    .route(
      "/v4/decodetracks",
      post(stats::decode_tracks),
    )
    .route(
      "/v4/sessions/{session_id}/players",
      get(player::get_players),
    )
    .route(
      "/v4/sessions/{session_id}/players/{guild_id}",
      get(player::get_player)
        .patch(player::update_player)
        .delete(player::destroy_player),
    )
    .route(
      "/v4/sessions/{session_id}",
      patch(player::update_session),
    )
    .route("/v4/routeplanner/status", get(stats::routeplanner_status))
    .route(
      "/v4/routeplanner/free/address",
      post(stats::routeplanner_free_address),
    )
    .route(
      "/v4/routeplanner/free/all",
      post(stats::routeplanner_free_all),
    )
    .layer(middleware::from_fn_with_state(state.clone(), check_auth))
    .with_state(state);

  Router::new()
    .merge(api_routes)
    .layer(middleware::from_fn(add_response_headers))
}
