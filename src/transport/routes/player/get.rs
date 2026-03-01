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
    match state.sessions.get(&session_id) {
        Some(session) => {
            let player_arcs: Vec<_> = session
                .players
                .iter()
                .map(|kv| kv.value().clone())
                .collect();
            let mut players = Vec::new();
            for arc in player_arcs {
                players.push(arc.read().await.to_player_response());
            }
            (StatusCode::OK, Json(Players { players })).into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(
                serde_json::to_value(crate::common::RustalinkError::not_found(
                    format!("Session not found: {}", session_id),
                    format!("/v4/sessions/{}/players", session_id),
                ))
                .unwrap(),
            ),
        )
            .into_response(),
    }
}

/// GET /v4/sessions/{sessionId}/players/{guildId}
pub async fn get_player(
    Path((session_id, guild_id)): Path<(
        crate::common::types::SessionId,
        crate::common::types::GuildId,
    )>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    tracing::info!("GET /v4/sessions/{}/players/{}", session_id, guild_id);
    match state.sessions.get(&session_id) {
        Some(session) => {
            let player_arc = session.players.get(&guild_id).map(|kv| kv.value().clone());
            match player_arc {
                Some(arc) => {
                    let player = arc.read().await;
                    (
                        StatusCode::OK,
                        Json(serde_json::to_value(player.to_player_response()).unwrap()),
                    )
                        .into_response()
                }
                None => (
                    StatusCode::NOT_FOUND,
                    Json(
                        serde_json::to_value(crate::common::RustalinkError::not_found(
                            format!("Player not found for guild: {}", guild_id),
                            format!("/v4/sessions/{}/players/{}", session_id, guild_id),
                        ))
                        .unwrap(),
                    ),
                )
                    .into_response(),
            }
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(
                serde_json::to_value(crate::common::RustalinkError::not_found(
                    format!("Session not found: {}", session_id),
                    format!("/v4/sessions/{}/players/{}", session_id, guild_id),
                ))
                .unwrap(),
            ),
        )
            .into_response(),
    }
}
