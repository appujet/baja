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

    match state.sessions.get(&session_id) {
        Some(session) => {
            if let Some((_, player_arc)) = session.players.remove(&guild_id) {
                let mut player = player_arc.write().await;
                // Emit TrackEnd(Cleanup) if track existed
                if player.track.is_some() {
                    if let Some(track_data) = player.to_player_response().track {
                        let end_event = protocol::OutgoingMessage::Event {
                            event: protocol::RustalinkEvent::TrackEnd {
                                guild_id: guild_id.clone(),
                                track: track_data,
                                reason: protocol::TrackEndReason::Cleanup,
                            },
                        };
                        session.send_message(&end_event);
                    }
                }

                // Abort background tasks
                if let Some(task) = player.track_task.take() {
                    task.abort();
                }
                if let Some(task) = player.gateway_task.take() {
                    task.abort();
                }
                if let Some(handle) = &player.track_handle {
                    player
                        .stop_signal
                        .store(true, std::sync::atomic::Ordering::SeqCst);
                    handle.stop();
                }
                {
                    let engine = player.engine.lock().await;
                    let mut mixer = engine.mixer.lock().await;
                    mixer.stop_all();
                }
            }
            StatusCode::NO_CONTENT.into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(
                serde_json::to_value(crate::common::RustalinkError::not_found(
                    "Session not found",
                    format!("/v4/sessions/{}/players/{}", session_id, guild_id),
                ))
                .unwrap(),
            ),
        )
            .into_response(),
    }
}
