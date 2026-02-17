use crate::audio::playback::{PlaybackState, TrackHandle};
use crate::types;
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
use dashmap::DashMap;
use serde::Deserialize;
use serde_json::Value;
use std::{num::NonZeroU64, sync::Arc};
use tokio::sync::Mutex;
use tracing::{error, info, warn};

pub type UserId = u64;

// ─── Shared State ───────────────────────────────────────────────────────────

/// Top-level application state.
pub struct AppState {
    pub sessions: DashMap<String, Arc<Session>>,
    /// Sessions disconnected but waiting for resume within timeout.
    pub resumable_sessions: DashMap<String, Arc<Session>>,
}

/// A single client session.
pub struct Session {
    pub session_id: String,
    pub user_id: Option<UserId>,
    pub players: DashMap<String, PlayerContext>,
    /// Sender for outgoing WS messages. Swapped on resume.
    pub sender: Mutex<flume::Sender<Message>>,
    pub resumable: std::sync::atomic::AtomicBool,
    pub resume_timeout: std::sync::atomic::AtomicU64,
    /// True when WS is disconnected but session is kept for resume.
    pub paused: std::sync::atomic::AtomicBool,
    /// Events queued while session is paused.
    pub event_queue: Mutex<Vec<String>>,
}

impl Session {
    /// Send a JSON message. If paused, queue it for replay.
    pub async fn send_json(&self, json: &str) {
        if self.paused.load(std::sync::atomic::Ordering::Relaxed) {
            let mut queue = self.event_queue.lock().await;
            queue.push(json.to_string());
        } else {
            let sender = self.sender.lock().await;
            let _ = sender.send(Message::Text(json.to_string().into()));
        }
    }

    /// Send a typed outgoing message.
    pub async fn send_message(&self, msg: &types::OutgoingMessage) {
        if let Ok(json) = serde_json::to_string(msg) {
            self.send_json(&json).await;
        }
    }
}

/// Internal player state.
pub struct PlayerContext {
    pub guild_id: String,
    pub volume: i32,
    pub paused: bool,
    pub track: Option<String>,
    pub track_handle: Option<TrackHandle>,
    pub position: u64,
    pub voice: VoiceConnectionState,
    pub engine: Arc<Mutex<crate::voice::VoiceEngine>>,
    pub filters: types::Filters,
    pub end_time: Option<u64>,
    pub stop_signal: Arc<std::sync::atomic::AtomicBool>,
}

#[derive(Clone, Default)]
pub struct VoiceConnectionState {
    pub token: String,
    pub endpoint: String,
    pub session_id: String,
    pub channel_id: Option<String>,
}

impl PlayerContext {
    pub fn new(guild_id: String) -> Self {
        Self {
            guild_id,
            volume: 100,
            paused: false,
            track: None,
            track_handle: None,
            position: 0,
            voice: VoiceConnectionState::default(),
            engine: Arc::new(Mutex::new(crate::voice::VoiceEngine::new())),
            filters: types::Filters::default(),
            end_time: None,
            stop_signal: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    pub fn to_player_response(&self) -> types::Player {
        let current_pos = self
            .track_handle
            .as_ref()
            .map(|h| h.get_position())
            .unwrap_or(self.position);

        types::Player {
            guild_id: self.guild_id.clone(),
            track: self.track.as_ref().map(|t| types::Track {
                encoded: BASE64_STANDARD.encode(t.as_bytes()),
                info: types::TrackInfo {
                    identifier: t.clone(),
                    is_seekable: true,
                    author: String::new(),
                    length: 0,
                    is_stream: false,
                    position: current_pos,
                    title: t.clone(),
                    uri: Some(t.clone()),
                    artwork_url: None,
                    isrc: None,
                    source_name: "http".to_string(),
                },
                plugin_info: serde_json::json!({}),
                user_data: serde_json::json!({}),
            }),
            volume: self.volume,
            paused: self.paused,
            state: types::PlayerState {
                time: now_ms(),
                position: current_pos,
                connected: !self.voice.token.is_empty(),
                ping: -1,
            },
            voice: types::VoiceState {
                token: self.voice.token.clone(),
                endpoint: self.voice.endpoint.clone(),
                session_id: self.voice.session_id.clone(),
                channel_id: self.voice.channel_id.clone(),
            },
            filters: self.filters.clone(),
        }
    }
}

// ─── WebSocket Message Types ────────────────────────────────────────────────

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

// ─── WebSocket Handler ──────────────────────────────────────────────────────

pub async fn websocket_handler(
    headers: HeaderMap,
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let user_id = headers
        .get("user-id")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .and_then(NonZeroU64::new)
        .map(UserId::from);

    let client_session_id = headers
        .get("session-id")
        .and_then(|h| h.to_str().ok())
        .map(String::from);

    ws.on_upgrade(move |socket| handle_socket(socket, state, user_id, client_session_id))
}

async fn handle_socket(
    mut socket: WebSocket,
    state: Arc<AppState>,
    user_id: Option<UserId>,
    client_session_id: Option<String>,
) {
    let (tx, rx) = flume::unbounded();

    // Check for session resume
    let (session, resumed) = if let Some(ref sid) = client_session_id {
        if let Some((_, existing)) = state.resumable_sessions.remove(sid) {
            // Resume: swap sender, replay events
            info!("Resuming session: {}", sid);

            existing
                .paused
                .store(false, std::sync::atomic::Ordering::Relaxed);

            // Swap in new sender
            {
                let mut sender = existing.sender.lock().await;
                *sender = tx.clone();
            }

            // Re-register in active sessions
            state
                .sessions
                .insert(sid.clone(), existing.clone());

            (existing, true)
        } else {
            // Session-Id provided but not found in resumable — create new
            let session_id = sid.clone();
            let session = create_session(session_id.clone(), user_id, tx.clone());
            state.sessions.insert(session_id, session.clone());
            (session, false)
        }
    } else {
        // New session
        let session_id = uuid::Uuid::new_v4().to_string();
        let session = create_session(session_id.clone(), user_id, tx.clone());
        state.sessions.insert(session_id, session.clone());
        (session, false)
    };

    let session_id = session.session_id.clone();
    info!(
        "WebSocket connected: session={} resumed={}",
        session_id, resumed
    );

    // Send Ready
    let ready = types::OutgoingMessage::Ready {
        resumed,
        session_id: session_id.clone(),
    };
    if let Ok(json) = serde_json::to_string(&ready) {
        let _ = socket.send(Message::Text(json.into())).await;
    }

    // If resumed, replay queued events and send player updates
    if resumed {
        let queued = {
            let mut queue = session.event_queue.lock().await;
            std::mem::take(&mut *queue)
        };
        info!(
            "Replaying {} queued events for session {}",
            queued.len(),
            session_id
        );
        for json in queued {
            let _ = socket.send(Message::Text(json.into())).await;
        }
        // Send fresh player updates for all players
        for player in session.players.iter() {
            let update = types::OutgoingMessage::PlayerUpdate {
                guild_id: player.guild_id.clone(),
                state: types::PlayerState {
                    time: now_ms(),
                    position: player
                        .track_handle
                        .as_ref()
                        .map(|h| h.get_position())
                        .unwrap_or(player.position),
                    connected: !player.voice.token.is_empty(),
                    ping: -1,
                },
            };
            if let Ok(json) = serde_json::to_string(&update) {
                let _ = socket.send(Message::Text(json.into())).await;
            }
        }
    }

    // Stats heartbeat — push every 60 seconds
    let session_for_stats = session.clone();
    let state_for_stats = state.clone();
    tokio::spawn(async move {
        let start = std::time::Instant::now();
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            if session_for_stats
                .paused
                .load(std::sync::atomic::Ordering::Relaxed)
            {
                continue; // Don't push stats while paused
            }
            let stats = collect_stats(&state_for_stats, start.elapsed().as_millis() as u64);
            let msg = types::OutgoingMessage::Stats(stats);
            session_for_stats.send_message(&msg).await;
        }
    });

    // Main event loop
    loop {
        tokio::select! {
            Ok(msg) = rx.recv_async() => {
                if let Err(e) = socket.send(msg).await {
                    error!("Socket send error: session={} err={}", session_id, e);
                    break;
                }
            }
            msg = socket.recv() => {
                let msg = match msg {
                    Some(Ok(msg)) => msg,
                    Some(Err(e)) => {
                        warn!("WebSocket error: session={} err={}", session_id, e);
                        break;
                    }
                    None => {
                        info!("WebSocket closed: session={}", session_id);
                        break;
                    }
                };

                match msg {
                    Message::Text(text) => {
                        match serde_json::from_str::<IncomingMessage>(&text) {
                            Ok(op) => {
                                tracing::debug!("WS op: session={} op={:?}", session_id, op);
                                let _ = handle_op(op, &state, &session_id).await;
                            }
                            Err(e) => {
                                warn!("Bad WS msg: session={} err={}", session_id, e);
                            }
                        }
                    }
                    Message::Close(_) => {
                        info!("WebSocket close: session={}", session_id);
                        break;
                    }
                    _ => {}
                }
            }
        }
    }

    // On disconnect: check if resumable
    let is_resumable = session
        .resumable
        .load(std::sync::atomic::Ordering::Relaxed);

    if is_resumable {
        info!(
            "Session paused for resume: session={}", session_id
        );
        session
            .paused
            .store(true, std::sync::atomic::Ordering::Relaxed);

        // Move from active to resumable
        state.sessions.remove(&session_id);
        state
            .resumable_sessions
            .insert(session_id.clone(), session.clone());

        // Start resume timeout
        let timeout_secs = session
            .resume_timeout
            .load(std::sync::atomic::Ordering::Relaxed);
        let state_cleanup = state.clone();
        let sid = session_id.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(timeout_secs)).await;
            // If still in resumable_sessions after timeout, destroy
            if state_cleanup.resumable_sessions.remove(&sid).is_some() {
                info!("Session resume timeout expired: {}", sid);
            }
        });
    } else {
        state.sessions.remove(&session_id);
        info!("Session destroyed: {}", session_id);
    }
}

fn create_session(session_id: String, user_id: Option<UserId>, tx: flume::Sender<Message>) -> Arc<Session> {
    Arc::new(Session {
        session_id,
        user_id,
        players: DashMap::new(),
        sender: Mutex::new(tx),
        resumable: std::sync::atomic::AtomicBool::new(false),
        resume_timeout: std::sync::atomic::AtomicU64::new(60),
        paused: std::sync::atomic::AtomicBool::new(false),
        event_queue: Mutex::new(Vec::new()),
    })
}

// ─── Op Handlers ────────────────────────────────────────────────────────────

async fn handle_op(
    op: IncomingMessage,
    state: &Arc<AppState>,
    session_id: &String,
) -> Result<(), String> {
    let session = match state.sessions.get(session_id) {
        Some(s) => s.clone(),
        None => return Err("Session not found".into()),
    };

    match op {
        IncomingMessage::VoiceUpdate {
            guild_id,
            session_id: voice_session_id,
            channel_id,
            event,
        } => {
            let user_id = match session.user_id {
                Some(uid) => uid,
                None => {
                    error!("No User-Id for voice: session={}", session.session_id);
                    return Ok(());
                }
            };

            let mut player = session
                .players
                .entry(guild_id.clone())
                .or_insert_with(|| PlayerContext::new(guild_id.clone()));

            player.voice.session_id = voice_session_id;
            player.voice.token = event["token"].as_str().unwrap_or("").to_string();
            player.voice.endpoint = event["endpoint"].as_str().unwrap_or("").to_string();
            player.voice.channel_id = channel_id;

            let engine = player.engine.clone();
            let guild = player.guild_id.clone();
            let voice = player.voice.clone();
            drop(player);

            connect_voice(engine, guild, user_id, voice).await?;
        }

        IncomingMessage::Play { guild_id, track } => {
            let mut player = session
                .players
                .entry(guild_id.clone())
                .or_insert_with(|| PlayerContext::new(guild_id.clone()));

            start_playback(&mut player, track, session.clone()).await;
        }

        IncomingMessage::Stop { guild_id } => {
            if let Some(mut player) = session.players.get_mut(&guild_id) {
                if let Some(handle) = &player.track_handle {
                    let _ = handle.stop();
                }
                player.track_handle = None;
                player.track = None;
            }
        }

        IncomingMessage::Destroy { guild_id } => {
            session.players.remove(&guild_id);
        }
    }
    Ok(())
}

// ─── Playback ───────────────────────────────────────────────────────────────

pub async fn start_playback(
    player: &mut PlayerContext,
    track: String,
    session: Arc<Session>,
) {
    // If a track is already presently assigned, this is a replacement operation.
    // Emit TrackEnd(Replaced) *before* stopping the old handle.
    if player.track.is_some() {
        let is_playing = if let Some(handle) = &player.track_handle {
            handle.get_state().await != PlaybackState::Stopped
        } else {
            false
        };

        if is_playing {
            if let Some(track_data) = player.to_player_response().track {
                let end_event = types::OutgoingMessage::Event(types::LavalinkEvent::TrackEnd {
                    guild_id: player.guild_id.clone(),
                    track: track_data.clone(),
                    reason: types::TrackEndReason::Replaced,
                });
                tracing::debug!("Emitting TrackEnd(Replaced) for guild {}", player.guild_id);
                session.send_message(&end_event).await;
            }
        }
    }

    if let Some(handle) = &player.track_handle {
        player
            .stop_signal
            .store(true, std::sync::atomic::Ordering::SeqCst);
        handle.stop().await;
    }

    player.track = Some(track.clone());
    player.position = 0;
    player.paused = false;

    // Create new stop signal for the new track.
    // The old signal (if any) is held by the old monitor task and will remain true.
    player.stop_signal = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let identifier = if let Ok(decoded) = BASE64_STANDARD.decode(&track) {
        String::from_utf8(decoded).unwrap_or(track.clone())
    } else {
        track.clone()
    };

    let source_manager = crate::sources::SourceManager::new();
    let playback_url = match source_manager.get_playback_url(&identifier).await {
        Some(url) => url,
        None => {
            error!("Failed to resolve URL: {}", identifier);
            return;
        }
    };

    info!("Playback: {} -> {}", identifier, playback_url);

    let rx = crate::player::start_decoding(playback_url);
    let (handle, audio_state, vol, pos) = TrackHandle::new();

    {
        let engine = player.engine.lock().await;
        let mut mixer = engine.mixer.lock().await;
        mixer.add_track(rx, audio_state, vol, pos);
    }

    player.track_handle = Some(handle);

    // Emit TrackStartEvent
    let track_data = player.to_player_response().track.unwrap();
    let start_event = types::OutgoingMessage::Event(types::LavalinkEvent::TrackStart {
        guild_id: player.guild_id.clone(),
        track: track_data.clone(),
    });
    session.send_message(&start_event).await;

    // Track monitor
    let guild_id = player.guild_id.clone();
    let handle_clone = player.track_handle.as_ref().unwrap().clone();
    let session_clone = session.clone();
    let stop_signal = player.stop_signal.clone();

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
        let mut last_update = std::time::Instant::now();

        loop {
            interval.tick().await;

            let current_state = handle_clone.get_state().await;
            if current_state == PlaybackState::Stopped {
                // Only emit TrackEnd(Finished) if this was a natural end, not a manual stop.
                // The handler emits TrackEnd(Stopped) for manual stops.
                if !stop_signal.load(std::sync::atomic::Ordering::SeqCst) {
                    tracing::debug!("Emitting TrackEnd(Finished) for guild {}", guild_id);
                    let end_event =
                        types::OutgoingMessage::Event(types::LavalinkEvent::TrackEnd {
                            guild_id: guild_id.clone(),
                            track: track_data.clone(),
                            reason: types::TrackEndReason::Finished,
                        });
                    session_clone.send_message(&end_event).await;
                }
                break;
            }

            if last_update.elapsed() >= std::time::Duration::from_secs(5) {
                last_update = std::time::Instant::now();
                let update = types::OutgoingMessage::PlayerUpdate {
                    guild_id: guild_id.clone(),
                    state: types::PlayerState {
                        time: now_ms(),
                        position: handle_clone.get_position(),
                        connected: true,
                        ping: -1,
                    },
                };
                session_clone.send_message(&update).await;
            }
        }
    });
}

// ─── Voice ──────────────────────────────────────────────────────────────────

pub async fn connect_voice(
    engine: Arc<Mutex<crate::voice::VoiceEngine>>,
    guild_id: String,
    user_id: UserId,
    voice: VoiceConnectionState,
) -> Result<(), String> {
    let engine_lock = engine.lock().await;
    let channel_id = voice
        .channel_id
        .as_ref()
        .and_then(|id| id.parse::<u64>().ok())
        .unwrap_or_else(|| guild_id.parse::<u64>().unwrap_or(0));

    let mixer = engine_lock.mixer.clone();
    let gateway = VoiceGateway::new(
        guild_id,
        user_id,
        channel_id,
        voice.session_id,
        voice.token,
        voice.endpoint,
        mixer,
    );

    tokio::spawn(async move {
        if let Err(e) = gateway.run().await {
            error!("Voice gateway error: {}", e);
        }
    });

    Ok(())
}

// ─── Helpers ────────────────────────────────────────────────────────────────

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Collect stats across all sessions.
pub fn collect_stats(state: &AppState, uptime: u64) -> types::Stats {
    let mut total_players = 0i32;
    let mut playing_players = 0i32;

    for session in state.sessions.iter() {
        for player in session.players.iter() {
            total_players += 1;
            if player.track.is_some() && !player.paused {
                playing_players += 1;
            }
        }
    }

    let (mem_used, mem_free, mem_total) = read_memory_stats();

    types::Stats {
        players: total_players,
        playing_players,
        uptime,
        memory: types::Memory {
            free: mem_free,
            used: mem_used,
            allocated: mem_used,  // RSS ≈ allocated for our purposes
            reservable: mem_total,
        },
        cpu: types::Cpu {
            cores: num_cpus(),
            system_load: 0.0,     // Requires periodic sampling — future enhancement
            lavalink_load: 0.0,
        },
        frame_stats: None,
    }
}

fn num_cpus() -> i32 {
    std::thread::available_parallelism()
        .map(|n| n.get() as i32)
        .unwrap_or(1)
}

/// Read process RSS from /proc/self/status and system free/total from /proc/meminfo.
fn read_memory_stats() -> (u64, u64, u64) {
    let rss = std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmRSS:"))
                .and_then(|l| {
                    l.split_whitespace()
                        .nth(1)
                        .and_then(|v| v.parse::<u64>().ok())
                })
                .map(|kb| kb * 1024) // Convert kB to bytes
        })
        .unwrap_or(0);

    let (mut total, mut free) = (0u64, 0u64);
    if let Ok(meminfo) = std::fs::read_to_string("/proc/meminfo") {
        for line in meminfo.lines() {
            if line.starts_with("MemTotal:") {
                total = line
                    .split_whitespace()
                    .nth(1)
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(0)
                    * 1024;
            } else if line.starts_with("MemAvailable:") {
                free = line
                    .split_whitespace()
                    .nth(1)
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(0)
                    * 1024;
            }
        }
    }

    (rss, free, total)
}

