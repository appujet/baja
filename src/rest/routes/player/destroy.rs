use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Json},
};

use crate::{protocol, server::AppState};

/// DELETE /v4/sessions/{sessionId}/players/{guildId}
pub async fn destroy_player(
    Path((session_id, guild_id)): Path<(
        crate::common::types::SessionId,
        crate::common::types::GuildId,
    )>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    tracing::info!("DELETE /v4/sessions/{}/players/{}", session_id, guild_id);

    let Some(session) = state.sessions.get(&session_id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(crate::common::RustalinkError::not_found(
                "Session not found",
                format!("/v4/sessions/{}/players/{}", session_id, guild_id),
            )),
        )
            .into_response();
    };

    if let Some(player_arc) = session.players.get(&guild_id).map(|kv| kv.value().clone()) {
        {
            let player = player_arc.read().await;
            if player.track.is_some()
                && let Some(track_data) = player.to_player_response().track
            {
                let end_event = protocol::OutgoingMessage::Event {
                    event: Box::new(protocol::RustalinkEvent::TrackEnd {
                        guild_id: guild_id.clone(),
                        track: track_data,
                        reason: protocol::TrackEndReason::Cleanup,
                    }),
                };
                session.send_message(&end_event);
            }
        }

        session.destroy_player(&guild_id, &state).await;
    }

    StatusCode::NO_CONTENT.into_response()
}
