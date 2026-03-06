use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Json},
};

use crate::{player::Players, server::AppState};

/// GET /v4/sessions/{sessionId}/players
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
        players.push(arc.read().await.to_player_response());
    }

    (StatusCode::OK, Json(Players { players })).into_response()
}

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
    (StatusCode::OK, Json(player.to_player_response())).into_response()
}
