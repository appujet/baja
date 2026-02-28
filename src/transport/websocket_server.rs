use std::sync::{Arc, atomic::Ordering};

use Ordering::Relaxed;
use axum::{
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use futures::{SinkExt, StreamExt};
use tracing::{debug, info, warn};

use crate::{
    common::{
        types::{SessionId, UserId},
        utils::now_ms,
    },
    monitoring::collect_stats,
    player::PlayerState,
    protocol,
    server::{AppState, Session},
};

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
        .and_then(std::num::NonZeroU64::new)
        .map(|n| UserId(n.get()));

    let user_id = match user_id {
        Some(uid) => uid,
        None => return Err((StatusCode::BAD_REQUEST, "Missing or invalid User-Id header")),
    };

    // 3. Client-Name Check (Optional, just logging)
    let client_name = headers.get("client-name").and_then(|h| h.to_str().ok());
    if let Some(name) = client_name {
        info!("Incoming connection from client: {}", name);
    } else {
        debug!("Client connected without 'Client-Name' header");
    }

    // 4. Session Resumption Check
    let client_session_id = headers
        .get("session-id")
        .and_then(|h| h.to_str().ok())
        .map(|s| SessionId(s.to_string()));

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
    client_session_id: Option<SessionId>,
) {
    let (tx, rx) = flume::unbounded();

    // Check for session resume
    let (session, resumed) = if let Some(ref sid) = client_session_id {
        if let Some((_, existing)) = state.resumable_sessions.remove(sid) {
            info!("Resuming session: {}", sid);
            existing.paused.store(false, Ordering::Relaxed);
            {
                let mut sender = existing.sender.lock();
                *sender = tx.clone();
            }
            state.sessions.insert(sid.clone(), existing.clone());
            (existing, true)
        } else {
            let session_id = SessionId::generate();
            let session = Arc::new(Session::new(
                session_id.clone(),
                Some(user_id),
                tx.clone(),
                state.config.server.max_event_queue_size,
            ));
            state.sessions.insert(session_id, session.clone());
            (session, false)
        }
    } else {
        let session_id = SessionId::generate();
        let session = Arc::new(Session::new(
            session_id.clone(),
            Some(user_id),
            tx.clone(),
            state.config.server.max_event_queue_size,
        ));
        state.sessions.insert(session_id, session.clone());
        (session, false)
    };

    let session_id = session.session_id.clone();
    info!(
        "WebSocket connected: session={} resumed={}",
        session_id, resumed
    );

    // Send Ready
    let ready = protocol::OutgoingMessage::Ready {
        resumed,
        session_id: session_id.clone(),
    };
    if let Ok(json) = serde_json::to_string(&ready) {
        let _ = socket.send(Message::Text(json.into())).await;
    }

    // Replay queued events if resumed
    if resumed {
        let queued = {
            let mut queue = session.event_queue.lock();
            std::mem::take(&mut *queue)
        };
        for json in queued {
            let _ = socket.send(Message::Text(json.into())).await;
        }

        let player_arcs: Vec<_> = session
            .players
            .iter()
            .map(|kv| kv.value().clone())
            .collect();
        let mut updates = Vec::new();
        for player_arc in player_arcs {
            let player = player_arc.read().await;
            let update = protocol::OutgoingMessage::PlayerUpdate {
                guild_id: player.guild_id.clone(),
                state: PlayerState {
                    time: now_ms(),
                    position: player
                        .track_handle
                        .as_ref()
                        .map(|h| h.get_position())
                        .unwrap_or(player.position),
                    connected: !player.voice.token.is_empty(),
                    ping: player.ping.load(Ordering::Relaxed),
                },
            };
            updates.push(update);
        }

        for update in updates {
            session.send_message(&update);
        }
    }

    let mut stats_interval = tokio::time::interval(std::time::Duration::from_secs(
        state.config.server.stats_interval,
    ));
    stats_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let mut ping_interval = tokio::time::interval(std::time::Duration::from_secs(
        state.config.server.websocket_ping_interval,
    ));
    ping_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let (mut ws_sink, mut ws_stream) = socket.split();
    let (ws_tx, mut ws_rx) = tokio::sync::mpsc::unbounded_channel::<Message>();

    // Spawn dedicated writer task to prevent HoL blocking
    let writer_session_id = session_id.clone();
    tokio::spawn(async move {
        while let Some(msg) = ws_rx.recv().await {
            if let Err(e) = ws_sink.send(msg).await {
                debug!(
                    "WebSocket writer task terminating for {}: {}",
                    writer_session_id, e
                );
                break;
            }
        }
    });

    loop {
        tokio::select! {
            _ = ping_interval.tick() => {
                if ws_tx.send(Message::Ping(b"heartbeat".to_vec().into())).is_err() {
                    break;
                }
            }
            _ = stats_interval.tick() => {
                if !session.paused.load(Relaxed) {
                    let stats = collect_stats(&state, Some(&session));
                    let msg = protocol::OutgoingMessage::Stats { stats };
                    if let Ok(json) = serde_json::to_string(&msg) {
                        if ws_tx.send(Message::Text(json.into())).is_err() {
                            break;
                        }
                    }
                }
            }
            res = rx.recv_async() => {
                match res {
                    Ok(msg) => {
                        if ws_tx.send(msg).is_err() {
                            break;
                        }
                    }
                    Err(_) => {
                        warn!("WebSocket session dropped (internal channel closed): session={}", session_id);
                        break;
                    }
                }
            }
            msg = ws_stream.next() => {
                let msg = match msg {
                    Some(Ok(msg)) => msg,
                    Some(Err(e)) => {
                        let err_msg = e.to_string();
                        if err_msg.contains("Connection reset") || err_msg.contains("Broken pipe") {
                            debug!("WebSocket connection closed abruptly by client: session={} err={}", session_id, e);
                        } else {
                            warn!("WebSocket error from client: session={} err={}", session_id, e);
                        }
                        break;
                    }
                    None => {
                        info!("WebSocket connection closed by client: session={}", session_id);
                        break;
                    }
                };

                match msg {
                    Message::Text(_) => {
                        warn!("Rustalink does not support WebSocket messages. Please use the REST API.");
                    }
                    Message::Ping(payload) => {
                        if ws_tx.send(Message::Pong(payload)).is_err() {
                             break;
                        }
                    }
                    Message::Pong(_) => {
                        debug!("Received heartbeat pong from client: session={}", session_id);
                    }
                    Message::Close(_) => {
                        info!("WebSocket received close frame: session={}", session_id);
                        break;
                    }
                    _ => {}
                }
            }
        }
    }

    if session.resumable.load(Relaxed) {
        session.paused.store(true, Relaxed);

        {
            let current_sender = session.sender.lock();
            if !current_sender.same_channel(&tx) {
                info!(
                    "Session {} replaced by a new connection; closing the old connection for cleanup.",
                    session_id
                );
                return;
            }
        }

        state.sessions.remove(&session_id);

        if let Some((_, removed)) = state.resumable_sessions.remove(&session_id) {
            warn!(
                "Shutting down resumable session {} because it shares an ID with a newly disconnected session.",
                removed.session_id
            );
            removed.shutdown();
        }

        state
            .resumable_sessions
            .insert(session_id.clone(), session.clone());

        let timeout_secs = session.resume_timeout.load(Relaxed);

        info!(
            "Connection closed (resumable). Session {} can be resumed within {} seconds.",
            session_id, timeout_secs
        );

        let state_cleanup = state.clone();
        let sid = session_id.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(timeout_secs)).await;
            if let Some((_, session)) = state_cleanup.resumable_sessions.remove(&sid) {
                warn!("Session resume timeout expired: {}", sid);
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
