use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Json},
};

use crate::{player::Players, protocol, server::AppState};

/// Returns the list of players for a session, sorted by `guild_id`.
///
/// If the session does not exist, responds with a 404 `RustalinkError::not_found`.
///
/// # Examples
///
/// ```rust
/// # use std::sync::Arc;
/// # use axum::extract::{Path, State};
/// # use axum::response::IntoResponse;
/// # async fn example(state: Arc<crate::AppState>, session_id: crate::common::types::SessionId) {
/// let resp = crate::handlers::get_players(Path(session_id), State(state)).await.into_response();
/// // inspect `resp` for a 200 OK with JSON `Players` or a 404 error
/// # }
/// ```
pub async fn get_players(
    Path(session_id): Path<crate::common::types::SessionId>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    tracing::info!("GET /v4/sessions/{}/players", session_id);

    let Some(session) = state.sessions.get(&session_id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(crate::common::RustalinkError::not_found(
                format!("Session not found: {}", session_id),
                format!("/v4/sessions/{}/players", session_id),
            )),
        )
            .into_response();
    };

    let mut players = Vec::new();
    for arc in session.players.iter().map(|kv| kv.value().clone()) {
        players.push(arc.read().await.to_player_response().await);
    }

    players.sort_by(|a, b| a.guild_id.cmp(&b.guild_id));

    (StatusCode::OK, Json(Players { players })).into_response()
}

/// GET /v4/sessions/{sessionId}
pub async fn get_session(
    Path(session_id): Path<crate::common::types::SessionId>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    tracing::info!("GET /v4/sessions/{}", session_id);

    let Some(session) = state.sessions.get(&session_id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(crate::common::RustalinkError::not_found(
                format!("Session not found: {}", session_id),
                format!("/v4/sessions/{}", session_id),
            )),
        )
            .into_response();
    };

    let info = protocol::SessionInfo {
        resuming: session.resumable.load(std::sync::atomic::Ordering::Relaxed),
        timeout: session
            .resume_timeout
            .load(std::sync::atomic::Ordering::Relaxed),
    };

    (StatusCode::OK, Json(info)).into_response()
}

/// Fetches a player by guild ID from a session and returns its serialized player response.
///
/// If the session identified by `session_id` does not exist, responds with HTTP 404 and a
/// `RustalinkError::not_found` describing the missing session and endpoint. If the session
/// exists but does not contain a player for `guild_id`, responds with HTTP 404 and a
/// `RustalinkError::not_found` describing the missing player and endpoint. If found, responds
/// with HTTP 200 and the player's JSON representation.
///
/// # Returns
///
/// An HTTP response: `200 OK` with the player's JSON when present; `404 Not Found` with a
/// `RustalinkError` JSON when the session or player is missing.
///
/// # Examples
///
/// ```no_run
/// use std::sync::Arc;
/// use axum::extract::{Path, State};
///
/// // given `state: Arc<AppState>`, `session_id`, and `guild_id`:
/// let response = get_player(Path((session_id, guild_id)), State(state)).await;
/// ```
pub async fn get_player(
    Path((session_id, guild_id)): Path<(
        crate::common::types::SessionId,
        crate::common::types::GuildId,
    )>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    tracing::info!("GET /v4/sessions/{}/players/{}", session_id, guild_id);

    let Some(session) = state.sessions.get(&session_id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(crate::common::RustalinkError::not_found(
                format!("Session not found: {}", session_id),
                format!("/v4/sessions/{}/players/{}", session_id, guild_id),
            )),
        )
            .into_response();
    };

    let Some(player_arc) = session.players.get(&guild_id).map(|kv| kv.value().clone()) else {
        return (
            StatusCode::NOT_FOUND,
            Json(crate::common::RustalinkError::not_found(
                format!("Player not found for guild: {}", guild_id),
                format!("/v4/sessions/{}/players/{}", session_id, guild_id),
            )),
        )
            .into_response();
    };

    let player = player_arc.read().await;
    (StatusCode::OK, Json(player.to_player_response().await)).into_response()
}
