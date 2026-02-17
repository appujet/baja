use std::sync::Arc;
use crate::server::{AppState, Session};
use crate::player::PlayerContext;
use crate::ws::messages::IncomingMessage;

pub async fn handle_op(
    op: IncomingMessage,
    state: &Arc<AppState>,
    session_id: &String,
) -> Result<(), String> {
    let session: Arc<Session> = match state.sessions.get(session_id) {
        Some(s) => s.clone(),
        None => return Err("Session not found".to_string()),
    };

    match op {
        IncomingMessage::VoiceUpdate {
            guild_id,
            session_id: voice_session_id,
            channel_id,
            event,
        } => {
            let token = event.get("token").and_then(|v| v.as_str()).ok_or("Missing token in voice update event")?.to_string();
            let endpoint = event.get("endpoint").and_then(|v| v.as_str()).ok_or("Missing endpoint in voice update event")?.to_string();

            if !session.players.contains_key(&guild_id) {
                session
                    .players
                    .insert(guild_id.clone(), PlayerContext::new(guild_id.clone()));
            }

            let mut player = session.players.get_mut(&guild_id).unwrap();
            player.voice = crate::player::VoiceConnectionState {
                token,
                endpoint,
                session_id: voice_session_id,
                channel_id,
            };

            if let Some(uid) = session.user_id {
                let engine = player.engine.clone();
                let guild = player.guild_id.clone();
                let voice_state = player.voice.clone();
                drop(player);
                let _ = crate::server::connect_voice(engine, guild, uid, voice_state).await;
            }
        }
        IncomingMessage::Play { guild_id, track } => {
            if !session.players.contains_key(&guild_id) {
                session
                    .players
                    .insert(guild_id.clone(), PlayerContext::new(guild_id.clone()));
            }

            let mut player = session.players.get_mut(&guild_id).unwrap();
            crate::server::start_playback(&mut player, track, session.clone()).await;
        }
        IncomingMessage::Stop { guild_id } => {
            if let Some(mut player) = session.players.get_mut(&guild_id) {
                if let Some(handle) = &player.track_handle {
                    let _ = handle.stop().await;
                }
                player.track = None;
                player.track_handle = None;
            }
        }
        IncomingMessage::Destroy { guild_id } => {
            session.players.remove(&guild_id);
        }
    }

    Ok(())
}
