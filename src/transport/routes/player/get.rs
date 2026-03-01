use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Json},
};

use crate::{
    common::utils::now_ms,
    player::{Filters, Player, PlayerState, Players, VoiceState},
    server::AppState,
};

/// GET /v4/sessions/{sessionId}/players
pub async fn get_players(
    Path(session_id): Path<crate::common::types::SessionId>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    tracing::info!("GET /v4/sessions/{}/players", session_id);
    match state.sessions.get(&session_id) {
        Some(session) => {
            let players: Vec<Player> = session
                .players
                .iter()
                .map(|p| p.to_player_response())
                .collect();
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
        Some(session) => match session.players.get(&guild_id) {
            Some(player) => (
                StatusCode::OK,
                Json(serde_json::to_value(player.to_player_response()).unwrap()),
            )
                .into_response(),
            None => {
                // Return empty player
                let empty = Player {
                    guild_id: guild_id.clone(),
                    track: None,
                    volume: 100,
                    paused: false,
                    state: PlayerState {
                        time: now_ms(),
                        position: 0,
                        connected: false,
                        ping: -1,
                    },
                    voice: VoiceState::default(),
                    filters: Filters::default(),
                };
                (StatusCode::OK, Json(serde_json::to_value(empty).unwrap())).into_response()
            }
        },
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
