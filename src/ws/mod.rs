use std::sync::Arc;
use axum::{
    extract::{State, ws::WebSocketUpgrade},
    http::HeaderMap,
    response::IntoResponse,
};
use std::num::NonZeroU64;
use crate::server::{AppState, UserId};

pub mod handler;
pub mod messages;
pub mod ops;

pub async fn websocket_handler(
    headers: HeaderMap,
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let user_id = headers
        .get("user-id")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .and_then(NonZeroU64::new)
        .map(UserId::from);

    let client_session_id = headers
        .get("session-id")
        .and_then(|h| h.to_str().ok())
        .map(String::from);

    ws.on_upgrade(move |socket| handler::handle_socket(socket, state, user_id, client_session_id))
}
