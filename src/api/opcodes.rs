use crate::playback::PlayerContext;
use crate::server::{AppState, Session};
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

#[derive(Deserialize, Debug)]
#[serde(tag = "op")]
#[serde(rename_all = "camelCase")]
pub enum IncomingMessage {
    VoiceUpdate {
        guild_id: String,
        session_id: String,
        channel_id: Option<String>,
        event: Value,
    },
    Play {
        guild_id: String,
        track: String,
    },
    Stop {
        guild_id: String,
    },
    Destroy {
        guild_id: String,
    },
}

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
            let token = event
                .get("token")
                .and_then(|v| v.as_str())
                .ok_or("Missing token in voice update event")?
                .to_string();
            let endpoint = event
                .get("endpoint")
                .and_then(|v| v.as_str())
                .ok_or("Missing endpoint in voice update event")?
                .to_string();

            if !session.players.contains_key(&guild_id) {
                session
                    .players
                    .insert(guild_id.clone(), PlayerContext::new(guild_id.clone()));
            }

            if let Some(uid) = session.user_id {
                let mut changed = false;
                {
                    let mut player = session.players.get_mut(&guild_id).unwrap();
                    if player.voice.token != token
                        || player.voice.endpoint != endpoint
                        || player.voice.session_id != voice_session_id
                        || player.voice.channel_id != channel_id
                    {
                        player.voice = crate::playback::VoiceConnectionState {
                            token,
                            endpoint,
                            session_id: voice_session_id,
                            channel_id,
                        };
                        changed = true;
                    }
                }

                if changed
                    || session
                        .players
                        .get(&guild_id)
                        .map(|p| p.gateway_task.is_none())
                        .unwrap_or(true)
                {
                    let mut player = session.players.get_mut(&guild_id).unwrap();
                    let engine = player.engine.clone();
                    let guild = player.guild_id.clone();
                    let voice_state = player.voice.clone();

                    if let Some(task) = player.gateway_task.take() {
                        task.abort();
                    }

                    drop(player);
                    let new_task =
                        crate::server::connect_voice(engine, guild, uid, voice_state).await;

                    if let Some(mut player) = session.players.get_mut(&guild_id) {
                        player.gateway_task = Some(new_task);
                    }
                }
            }
        }
        IncomingMessage::Play { guild_id, track } => {
            if !session.players.contains_key(&guild_id) {
                session
                    .players
                    .insert(guild_id.clone(), PlayerContext::new(guild_id.clone()));
            }

            let mut player = session.players.get_mut(&guild_id).unwrap();
            crate::server::start_playback(
                &mut player,
                track,
                session.clone(),
                state.source_manager.clone(),
                state.routeplanner.clone(),
            )
            .await;
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
            if let Some((_, mut player)) = session.players.remove(&guild_id) {
                if let Some(task) = player.gateway_task.take() {
                    task.abort();
                }
            }
        }
    }

    Ok(())
}
