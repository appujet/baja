use std::sync::Arc;

use axum::{
    extract::{Request, State},
    http::{HeaderValue, StatusCode},
    middleware::Next,
    response::Response,
};
use tracing::warn;

use crate::server::AppState;

pub async fn check_auth(
    State(state): State<Arc<AppState>>,
    req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let auth_header = req
        .headers()
        .get("authorization")
        .and_then(|h| h.to_str().ok());

    match auth_header {
        Some(auth) if auth == state.config.server.authorization => Ok(next.run(req).await),
        Some(_) => {
            warn!("REST authorization failed: invalid password");
            Err(StatusCode::UNAUTHORIZED)
        }
        None => {
            warn!("REST authorization failed: missing authorization header");
            Err(StatusCode::UNAUTHORIZED)
        }
    }
}

pub async fn add_response_headers(req: Request, next: Next) -> Response {
    let mut response = next.run(req).await;
    response
        .headers_mut()
        .insert("Rustalink-Api-Version", HeaderValue::from_static("4"));
    response
}
