use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Json},
};

use crate::{player::Players, protocol, server::AppState};

/// GET /v4/sessions/{sessionId}/players
pub async fn get_players(
    Path(session_id): Path<crate::common::types::SessionId>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    tracing::info!("GET /v4/sessions/{}/players", session_id);

    let player_arcs: Vec<_> = {
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
        session
            .players
            .iter()
            .map(|kv| kv.value().clone())
            .collect()
    };

    let mut players = Vec::new();
    for arc in player_arcs {
        players.push(crate::player::PlayerContext::to_response(arc).await);
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

pub async fn get_player(
    Path((session_id, guild_id)): Path<(
        crate::common::types::SessionId,
        crate::common::types::GuildId,
    )>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    tracing::info!("GET /v4/sessions/{}/players/{}", session_id, guild_id);

    let player_arc = {
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

        session.players.get(&guild_id).map(|kv| kv.value().clone())
    };

    let Some(arc) = player_arc else {
        return (
            StatusCode::NOT_FOUND,
            Json(crate::common::RustalinkError::not_found(
                format!("Player not found for guild: {}", guild_id),
                format!("/v4/sessions/{}/players/{}", session_id, guild_id),
            )),
        )
            .into_response();
    };

    (
        StatusCode::OK,
        Json(crate::player::PlayerContext::to_response(arc).await),
    )
        .into_response()
}
