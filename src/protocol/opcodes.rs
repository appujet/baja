use std::sync::Arc;

use serde::Deserialize;
use serde_json::Value;

use crate::{
    player::{PlayerContext, VoiceConnectionState},
    server::{AppState, Session},
};

#[derive(Deserialize, Debug)]
#[serde(tag = "op")]
#[serde(rename_all = "camelCase")]
pub enum IncomingMessage {
    VoiceUpdate {
        guild_id: crate::common::types::GuildId,
        session_id: String,
        channel_id: Option<String>,
        event: Value,
    },
    Play {
        guild_id: crate::common::types::GuildId,
        track: String,
    },
    Stop {
        guild_id: crate::common::types::GuildId,
    },
    Destroy {
        guild_id: crate::common::types::GuildId,
    },
}

pub async fn handle_op(
    op: IncomingMessage,
    state: &Arc<AppState>,
    session_id: &crate::common::types::SessionId,
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
                session.players.insert(
                    guild_id.clone(),
                    PlayerContext::new(guild_id.clone(), &state.config.player),
                );
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
                        player.voice = VoiceConnectionState {
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
                    let filter_chain = player.filter_chain.clone();
                    let ping = player.ping.clone();

                    if let Some(task) = player.gateway_task.take() {
                        task.abort();
                    }

                    let frames_sent = player.frames_sent.clone();
                    let frames_nulled = player.frames_nulled.clone();

                    drop(player);
                    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
                    let session_clone = session.clone();
                    tokio::spawn(async move {
                        while let Some(event) = event_rx.recv().await {
                            let msg = crate::protocol::OutgoingMessage::Event { event: event };
                            session_clone.send_message(&msg);
                        }
                    });
                    let new_task = crate::server::connect_voice(
                        engine,
                        guild,
                        uid,
                        voice_state,
                        filter_chain,
                        ping,
                        Some(event_tx),
                        frames_sent,
                        frames_nulled,
                    )
                    .await;

                    if let Some(mut player) = session.players.get_mut(&guild_id) {
                        player.gateway_task = Some(new_task);
                    }
                }
            }
        }
        IncomingMessage::Play { guild_id, track } => {
            if !session.players.contains_key(&guild_id) {
                session.players.insert(
                    guild_id.clone(),
                    PlayerContext::new(guild_id.clone(), &state.config.player),
                );
            }

            let mut player = session.players.get_mut(&guild_id).unwrap();
            crate::player::start_playback(
                &mut player,
                track,
                session.clone(),
                state.source_manager.clone(),
                state.lyrics_manager.clone(),
                state.routeplanner.clone(),
                state.config.server.player_update_interval,
                None,
                None, // end_time: not supplied via legacy opcode
            )
            .await;
        }
        IncomingMessage::Stop { guild_id } => {
            if let Some(mut player) = session.players.get_mut(&guild_id) {
                if let Some(handle) = &player.track_handle {
                    handle.stop();
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
