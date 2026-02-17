use crate::audio::playback::{PlaybackState, TrackHandle};
use crate::voice::VoiceGateway;
use axum::{
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::HeaderMap,
    response::IntoResponse,
};
use base64::prelude::*;
use serde::Deserialize;
use serde_json::Value;
use std::{collections::HashMap, num::NonZeroU64, sync::Arc};
pub type UserId = u64;
pub type GuildId = u64;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

// Shared state
pub struct AppState {
    pub sessions: Mutex<HashMap<String, Session>>,
}

pub struct Session {
    #[allow(dead_code)]
    pub players: Mutex<HashMap<String, PlayerState>>, // Guild ID -> Player
    pub sender: flume::Sender<Message>,
    pub user_id: Option<UserId>,
}

use crate::voice::VoiceEngine;

#[derive(Clone, Default)]
pub struct VoiceState {
    pub token: String,
    pub endpoint: String,
    pub session_id: String,
    pub channel_id: Option<String>,
}

#[allow(dead_code)]
#[derive(Clone)]
pub struct PlayerState {
    pub guild_id: String,
    pub volume: u32,
    pub paused: bool,
    pub track: Option<String>,
    pub position: u64,
    pub voice: VoiceState,
    pub track_handle: Option<TrackHandle>,
    pub engine: Arc<Mutex<VoiceEngine>>,
}

#[derive(serde::Serialize, Debug)]
#[serde(tag = "op")]
#[serde(rename_all = "camelCase")]
pub enum OutgoingMessage {
    Ready {
        resumed: bool,
        #[serde(rename = "sessionId")]
        session_id: String,
    },
    PlayerUpdate {
        guild_id: String,
        state: PlayerUpdateState,
    },
    #[serde(rename = "stats")]
    Stats(Stats),
    #[serde(rename = "event")]
    Event {
        #[serde(flatten)]
        event: PlayerEvent,
    },
}

#[derive(serde::Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub players: i32,
    pub playing_players: i32,
    pub uptime: u64,
    pub memory: MemoryStats,
    pub cpu: CpuStats,
    pub frame_stats: Option<FrameStats>,
}

#[derive(serde::Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MemoryStats {
    pub free: u64,
    pub used: u64,
    pub allocated: u64,
    pub reservable: u64,
}

#[derive(serde::Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CpuStats {
    pub cores: i32,
    pub system_load: f64,
    pub lavalink_load: f64,
}

#[derive(serde::Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FrameStats {
    pub sent: i32,
    pub nulled: i32,
    pub deficit: i32,
}

#[derive(serde::Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PlayerUpdateState {
    pub time: u64,
    pub position: u64,
    pub connected: bool,
    pub ping: i32,
}

#[derive(serde::Serialize, Debug)]
#[serde(tag = "type")]
pub enum PlayerEvent {
    #[serde(rename = "TrackStartEvent")]
    TrackStartEvent {
        #[serde(rename = "guildId")]
        guild_id: String,
        track: String,
    },
    #[serde(rename = "TrackEndEvent")]
    TrackEndEvent {
        #[serde(rename = "guildId")]
        guild_id: String,
        track: String,
        reason: String,
    },
    #[serde(rename = "TrackExceptionEvent")]
    TrackExceptionEvent {
        #[serde(rename = "guildId")]
        guild_id: String,
        track: String,
        exception: Exception,
    },
}

#[derive(serde::Serialize, Debug)]
pub struct Exception {
    pub message: String,
    pub severity: String,
    pub cause: String,
}

#[derive(Deserialize, Debug)]
#[serde(tag = "op")]
#[serde(rename_all = "camelCase")]
pub enum IncomingMessage {
    VoiceUpdate {
        guild_id: String,
        session_id: String,
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

pub async fn websocket_handler(
    headers: HeaderMap,
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let user_id = headers
        .get("user-id")
        .and_then(|h| h.to_str().ok())
        .map(|s| s.parse::<u64>().ok())
        .flatten()
        .and_then(NonZeroU64::new)
        .map(UserId::from);

    ws.on_upgrade(move |socket| handle_socket(socket, state, user_id))
}

async fn handle_socket(mut socket: WebSocket, state: Arc<AppState>, user_id: Option<UserId>) {
    let session_id = uuid::Uuid::new_v4().to_string();
    let (tx, rx) = flume::unbounded();

    // Register session
    {
        let mut sessions = state.sessions.lock().await;
        sessions.insert(
            session_id.clone(),
            Session {
                players: Mutex::new(HashMap::new()),
                sender: tx.clone(),
                user_id,
            },
        );
    }

    info!("New WebSocket connection: {}", session_id);

    // Send Ready op
    let ready = OutgoingMessage::Ready {
        resumed: false,
        session_id: session_id.clone(),
    };

    if let Ok(json) = serde_json::to_string(&ready) {
        let _ = socket.send(Message::Text(json.into())).await;
    }

    // Stats heartbeat
    let tx_stats = tx.clone();
    tokio::spawn(async move {
        let mut uptime = 0;
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            uptime += 30;
            let stats = OutgoingMessage::Stats(Stats {
                players: 0,         // TODO: track active players
                playing_players: 0, // TODO: track active players
                uptime,
                memory: MemoryStats {
                    free: 1024 * 1024 * 100,
                    used: 1024 * 1024 * 50,
                    allocated: 1024 * 1024 * 150,
                    reservable: 1024 * 1024 * 200,
                },
                cpu: CpuStats {
                    cores: 1,
                    system_load: 0.1,
                    lavalink_load: 0.05,
                },
                frame_stats: None,
            });

            if let Ok(json) = serde_json::to_string(&stats) {
                if tx_stats.send(Message::Text(json.into())).is_err() {
                    break;
                }
            }
        }
    });

    loop {
        tokio::select! {
            // Outgoing messages
            Ok(msg) = rx.recv_async() => {
                if let Err(e) = socket.send(msg).await {
                    error!("Socket send error for session {}: {}", session_id, e);
                    break;
                }
            }
            // Incoming messages
            msg = socket.recv() => {
                let msg = match msg {
                    Some(Ok(msg)) => msg,
                    Some(Err(e)) => {
                        warn!("WebSocket error for {}: {}", session_id, e);
                        break;
                    }
                    None => {
                        info!("WebSocket closed for {}", session_id);
                        break;
                    }
                };

                match msg {
                    Message::Text(text) => match serde_json::from_str::<IncomingMessage>(&text) {
                        Ok(op) => {
                            info!("Received op for {}: {:?}", session_id, op);
                            let _ = handle_op(op, &state, &session_id).await;
                        }
                        Err(e) => {
                            warn!("Failed to parse message for {}: {} | Text: {}", session_id, e, text);
                        }
                    },
                    Message::Close(_) => {
                        info!("WebSocket closed for {}", session_id);
                        break;
                    }
                    _ => {}
                }
            }
        }
    }

    // Cleanup session on disconnect
    let mut sessions = state.sessions.lock().await;
    sessions.remove(&session_id);
    info!("Cleaned up session {}", session_id);
}

async fn handle_op(
    op: IncomingMessage,
    state: &Arc<AppState>,
    session_id: &String,
) -> Result<(), String> {
    match op {
        IncomingMessage::VoiceUpdate {
            guild_id,
            session_id: voice_session_id,
            event,
        } => {
            info!(
                "Voice update for guild {}: session={}, event={:?}",
                guild_id, voice_session_id, event
            );

            let sessions = state.sessions.lock().await;
            if let Some(session) = sessions.get(session_id) {
                // We need user_id to connect
                let user_id = match session.user_id {
                    Some(uid) => uid,
                    None => {
                        error!(
                            "Cannot connect to voice: Missing User-Id header in session {}",
                            session_id
                        );
                        return Ok(());
                    }
                };

                let mut players = session.players.lock().await;

                let player = players
                    .entry(guild_id.clone())
                    .or_insert_with(|| PlayerState {
                        guild_id: guild_id.clone(),
                        volume: 100,
                        paused: false,
                        track: None,
                        position: 0,
                        voice: VoiceState::default(),
                        track_handle: None,
                        engine: Arc::new(Mutex::new(VoiceEngine::new())),
                    });

                // Update voice state
                player.voice.session_id = voice_session_id.clone();
                player.voice.token = event["token"].as_str().unwrap_or("").to_string();
                player.voice.endpoint = event["endpoint"].as_str().unwrap_or("").to_string();

                let _ = connect_player(player, user_id).await;
            }
        }
        IncomingMessage::Play { guild_id, track } => {
            info!("Play request for guild {}: {}", guild_id, track);

            let sessions = state.sessions.lock().await;
            if let Some(session) = sessions.get(session_id) {
                let mut players = session.players.lock().await;

                let player = players
                    .entry(guild_id.clone())
                    .or_insert_with(|| PlayerState {
                        guild_id: guild_id.clone(),
                        volume: 100,
                        paused: false,
                        track: None,
                        position: 0,
                        voice: VoiceState::default(),
                        track_handle: None,
                        engine: Arc::new(Mutex::new(VoiceEngine::new())),
                    });

                start_playback(player, track, session.sender.clone()).await;
            }
        }
        IncomingMessage::Stop { guild_id } => {
            info!("Stop request for guild {}", guild_id);
            let sessions = state.sessions.lock().await;
            if let Some(session) = sessions.get(session_id) {
                let mut players = session.players.lock().await;
                if let Some(player) = players.get_mut(&guild_id) {
                    if let Some(handle) = &player.track_handle {
                        let _ = handle.stop();
                    }
                    player.track_handle = None;
                    player.track = None;

                    // We also should remove the player from map or keep it? Lavalink usually keeps the player but stops track.
                    // But previous code removed it. Let's keep it but reset track.
                }
            }
        }
        IncomingMessage::Destroy { guild_id } => {
            info!("Destroy request for guild {}", guild_id);
            let sessions = state.sessions.lock().await;
            if let Some(session) = sessions.get(session_id) {
                let mut players = session.players.lock().await;
                let _ = players.remove(&guild_id);
            }
        }
    }
    Ok(())
}

pub async fn start_playback(
    player: &mut PlayerState,
    track: String,
    sender: flume::Sender<Message>,
) {
    if let Some(handle) = &player.track_handle {
        let _ = handle.stop();
    }

    player.track = Some(track.clone());
    player.position = 0;
    player.paused = false;

    let url = if let Ok(decoded) = BASE64_STANDARD.decode(&track) {
        String::from_utf8(decoded).unwrap_or(track.clone())
    } else {
        track.clone()
    };

    if url.starts_with("http") {
        let rx = crate::player::start_decoding(url);
        let (handle, state, vol, pos) = TrackHandle::new();

        {
            let engine = player.engine.lock().await;
            let mut mixer = engine.mixer.lock().await;
            mixer.add_track(rx, state, vol, pos);
        }

        player.track_handle = Some(handle);

        let event = OutgoingMessage::Event {
            event: PlayerEvent::TrackStartEvent {
                guild_id: player.guild_id.clone(),
                track: track.clone(),
            },
        };
        if let Ok(json) = serde_json::to_string(&event) {
            let _ = sender.send(Message::Text(json.into()));
        }

        // Spawn a task to monitor this specific track and send updates/end event
        let tx_clone = sender.clone();
        let guild_id_clone = player.guild_id.clone();
        let track_clone = track.clone();
        let handle_clone = player.track_handle.as_ref().unwrap().clone();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
            let mut last_update = std::time::Instant::now();

            loop {
                interval.tick().await;

                let current_state = handle_clone.get_state().await;
                if current_state == PlaybackState::Stopped {
                    info!(
                        "Sending TrackEndEvent for guild {}: {}",
                        guild_id_clone, track_clone
                    );
                    let event = OutgoingMessage::Event {
                        event: PlayerEvent::TrackEndEvent {
                            guild_id: guild_id_clone,
                            track: track_clone,
                            reason: "FINISHED".to_string(),
                        },
                    };
                    if let Ok(json) = serde_json::to_string(&event) {
                        let _ = tx_clone.send(Message::Text(json.into()));
                    }
                    break;
                }

                if last_update.elapsed() >= std::time::Duration::from_secs(5) {
                    last_update = std::time::Instant::now();
                    let update = OutgoingMessage::PlayerUpdate {
                        guild_id: guild_id_clone.clone(),
                        state: PlayerUpdateState {
                            time: std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap()
                                .as_millis() as u64,
                            position: handle_clone.get_position(),
                            connected: true,
                            ping: 0,
                        },
                    };
                    if let Ok(json) = serde_json::to_string(&update) {
                        if tx_clone.send(Message::Text(json.into())).is_err() {
                            break;
                        }
                    }
                }
            }
        });
    }
}

pub async fn connect_player(player: &mut PlayerState, user_id: UserId) -> Result<(), String> {
    let mut engine = player.engine.lock().await;

    let guild_id_u64 = player.guild_id.parse::<u64>().unwrap_or(0);
    let channel_id = player
        .voice
        .channel_id
        .as_ref()
        .and_then(|id| id.parse::<u64>().ok())
        .unwrap_or(guild_id_u64);

    let mixer = engine.mixer.clone();
    let gateway = VoiceGateway::new(
        player.guild_id.clone(),
        user_id,
        channel_id,
        player.voice.session_id.clone(),
        player.voice.token.clone(),
        player.voice.endpoint.clone(),
        mixer,
    );

    tokio::spawn(async move {
        if let Err(e) = gateway.run().await {
            error!("Voice gateway error: {}", e);
        }
    });

    Ok(())
}
