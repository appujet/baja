mod handler;

use std::sync::Arc;

use axum::{
    extract::{State, ws::WebSocketUpgrade},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
pub use handler::handle_socket;
use tracing::{debug, info, warn};

use crate::{
    common::types::{SessionId, UserId},
    server::AppState,
};

pub async fn websocket_handler(
    headers: HeaderMap,
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> Result<Response, (StatusCode, &'static str)> {
    // 1. Authorization Check
    let auth_header = headers.get("authorization").and_then(|h| h.to_str().ok());
    let Some(auth) = auth_header else {
        warn!("Authorization failed: Missing Authorization header");
        return Err((StatusCode::UNAUTHORIZED, "Unauthorized"));
    };

    if auth != state.config.server.authorization {
        warn!("Authorization failed: Invalid password provided");
        return Err((StatusCode::UNAUTHORIZED, "Unauthorized"));
    }

    // 2. User-Id Check
    let user_id = headers
        .get("user-id")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .and_then(std::num::NonZeroU64::new)
        .map(|n| UserId(n.get()));

    // 3. Client-Name Check
    if let Some(name) = headers.get("client-name").and_then(|h| h.to_str().ok()) {
        info!("Incoming connection from client: {name}");
    } else {
        debug!("Client connected without 'Client-Name' header");
    }

    // 4. Session Resumption Check
    let client_session_id = headers
        .get("session-id")
        .and_then(|h| h.to_str().ok())
        .map(|s| SessionId(s.to_string()));

    let resuming = client_session_id
        .as_ref()
        .is_some_and(|sid| state.resumable_sessions.contains_key(sid));

    // 5. Upgrade and set headers
    let upgrade_callback = move |socket| handle_socket(socket, state, user_id, client_session_id);
    let mut response = ws.on_upgrade(upgrade_callback).into_response();

    response
        .headers_mut()
        .insert("Session-Resumed", resuming.to_string().parse().unwrap());
    response
        .headers_mut()
        .insert("Lavalink-Major-Version", "4".parse().unwrap());

    Ok(response)
}
