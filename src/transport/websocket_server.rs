use crate::api;
use crate::monitoring::collect_stats;
use crate::playback::PlayerState;
use crate::server::now_ms;
use crate::server::{AppState, Session, UserId};
use axum::{
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use std::num::NonZeroU64;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{error, info, warn};
use std::sync::atomic::Ordering::Relaxed;

pub async fn websocket_handler(
    headers: HeaderMap,
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> Result<Response, (StatusCode, &'static str)> {
    // 1. Authorization Check
    let auth_header = headers.get("authorization").and_then(|h| h.to_str().ok());

    match auth_header {
        Some(auth) if auth == state.config.server.password => {}
        Some(_) => {
            warn!("Authorization failed: Invalid password provided");
            return Err((StatusCode::UNAUTHORIZED, "Unauthorized"));
        }
        None => {
            warn!("Authorization failed: Missing Authorization header");
            return Err((StatusCode::UNAUTHORIZED, "Unauthorized"));
        }
    }

    // 2. User-Id Check
    let user_id = headers
        .get("user-id")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .and_then(NonZeroU64::new)
        .map(UserId::from);

    let user_id = match user_id {
        Some(uid) => uid,
        None => return Err((StatusCode::BAD_REQUEST, "Missing or invalid User-Id header")),
    };

    // 3. Client-Name Check (Optional, just logging)
    let client_name = headers.get("client-name").and_then(|h| h.to_str().ok());
    if let Some(name) = client_name {
        info!("Incoming connection from client: {}", name);
    } else {
        warn!("Client connected without 'Client-Name' header");
    }

    // 4. Session Resumption Check
    let client_session_id = headers
        .get("session-id")
        .and_then(|h| h.to_str().ok())
        .map(String::from);

    let resuming = if let Some(ref sid) = client_session_id {
        state.resumable_sessions.contains_key(sid)
    } else {
        false
    };

    // 5. Upgrade and set headers
    let upgrade_callback = move |socket| handle_socket(socket, state, user_id, client_session_id);
    let mut response = ws.on_upgrade(upgrade_callback).into_response();

    response
        .headers_mut()
        .insert("Session-Resumed", resuming.to_string().parse().unwrap());
    response
        .headers_mut()
        .insert("Lavalink-Major-Version", "4".parse().unwrap());

    Ok(response)
}

pub async fn handle_socket(
    mut socket: WebSocket,
    state: Arc<AppState>,
    user_id: UserId,
    client_session_id: Option<String>,
) {
    let (tx, rx) = flume::unbounded();

    // Check for session resume
    let (session, resumed) = if let Some(ref sid) = client_session_id {
        if let Some((_, existing)) = state.resumable_sessions.remove(sid) {
            info!("Resuming session: {}", sid);
            existing
                .paused
                .store(false, std::sync::atomic::Ordering::Relaxed);
            {
                let mut sender = existing.sender.lock().await;
                *sender = tx.clone();
            }
            state.sessions.insert(sid.clone(), existing.clone());
            (existing, true)
        } else {
            // Session ID provided but not found -> New Session
            let session_id = uuid::Uuid::new_v4().to_string();
            let session = create_session(session_id.clone(), Some(user_id), tx.clone());
            state.sessions.insert(session_id, session.clone());
            (session, false)
        }
    } else {
        // No Session ID provided -> New Session
        let session_id = uuid::Uuid::new_v4().to_string();
        let session = create_session(session_id.clone(), Some(user_id), tx.clone());
        state.sessions.insert(session_id, session.clone());
        (session, false)
    };

    let session_id = session.session_id.clone();
    info!(
        "WebSocket connected: session={} resumed={}",
        session_id, resumed
    );

    // Send Ready
    let ready = api::OutgoingMessage::Ready {
        resumed,
        session_id: session_id.clone(),
    };
    if let Ok(json) = serde_json::to_string(&ready) {
        let _ = socket.send(Message::Text(json.into())).await;
    }

    // If resumed, replay queued events
    if resumed {
        let queued = {
            let mut queue = session.event_queue.lock().await;
            std::mem::take(&mut *queue)
        };
        for json in queued {
            let _ = socket.send(Message::Text(json.into())).await;
        }
        for player in session.players.iter() {
            let update = api::OutgoingMessage::PlayerUpdate {
                guild_id: player.guild_id.clone(),
                state: PlayerState {
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

    // Interval for stats heartbeat
    let mut stats_interval = tokio::time::interval(std::time::Duration::from_secs(60));
    stats_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let start_time = std::time::Instant::now();

    // Main event loop
    loop {
        tokio::select! {
            _ = stats_interval.tick() => {
                 // Send stats only if not paused and this socket is active
                 if !session.paused.load(Relaxed) {
                     let stats = collect_stats(&state, start_time.elapsed().as_millis() as u64);
                     let msg = api::OutgoingMessage::Stats(stats);
                     if let Ok(json) = serde_json::to_string(&msg) {
                        if let Err(e) = socket.send(Message::Text(json.into())).await {
                             error!("Socket send error (stats): session={} err={}", session_id, e);
                             break;
                        }
                     }
                 }
            }
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
                    None => break,
                };

                match msg {
                    Message::Text(_) => {
                        warn!("Lavalink v4 does not support websocket messages. Please use the REST api.");
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
        }
    }

    // Cleanup or pause for resume
    if session.resumable.load(Relaxed) {
        session
            .paused
            .store(true, Relaxed);

        // Race condition check: Ensure we haven't been replaced by a new connection
        {
            let current_sender = session.sender.lock().await;
            if !current_sender.same_channel(&tx) {
                info!(
                    "Session {} replaced by new connection, closing old connection cleanup.",
                    session_id
                );
                return;
            }
        }

        state.sessions.remove(&session_id);

        // "Shutdown resumable session with id ... because it has the same id as a newly disconnected resumable session."
        if let Some((_, removed)) = state.resumable_sessions.remove(&session_id) {
            warn!(
                "Shutdown resumable session with id {} because it has the same id as a newly disconnected resumable session.",
                removed.session_id
            );
            removed.shutdown();
        }

        state
            .resumable_sessions
            .insert(session_id.clone(), session.clone());

        let timeout_secs = session
            .resume_timeout
            .load(Relaxed);

        info!(
            "Connection closed (resumable). Session {} can be resumed within {} seconds.",
            session_id, timeout_secs
        );

        let state_cleanup = state.clone();
        let sid = session_id.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(timeout_secs)).await;
            // If the session is still in resumable_sessions, it means it wasn't resumed.
            if let Some((_, session)) = state_cleanup.resumable_sessions.remove(&sid) {
                info!("Session resume timeout expired: {}", sid);
                session.shutdown();
            }
        });
    } else {
        if let Some((_, session)) = state.sessions.remove(&session_id) {
            info!("Connection closed (not resumable): {}", session_id);
            session.shutdown();
        }
    }
}

fn create_session(
    session_id: String,
    user_id: Option<UserId>,
    tx: flume::Sender<Message>,
) -> Arc<Session> {
    Arc::new(Session {
        session_id,
        user_id,
        players: dashmap::DashMap::new(),
        sender: Mutex::new(tx),
        resumable: std::sync::atomic::AtomicBool::new(false),
        resume_timeout: std::sync::atomic::AtomicU64::new(60),
        paused: std::sync::atomic::AtomicBool::new(false),
        event_queue: Mutex::new(Vec::new()),
    })
}
