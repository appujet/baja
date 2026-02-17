use std::sync::Arc;
use axum::extract::ws::{Message, WebSocket};
use tracing::{error, info, warn};
use crate::server::{AppState, Session, UserId};
use crate::types;
use crate::player::PlayerState;
use crate::ws::messages::IncomingMessage;
use crate::ws::ops::handle_op;
use crate::server::{now_ms, collect_stats};
use tokio::sync::Mutex;

pub async fn handle_socket(
    mut socket: WebSocket,
    state: Arc<AppState>,
    user_id: Option<UserId>,
    client_session_id: Option<String>,
) {
    let (tx, rx) = flume::unbounded();

    // Check for session resume
    let (session, resumed) = if let Some(ref sid) = client_session_id {
        if let Some((_, existing)) = state.resumable_sessions.remove(sid) {
            info!("Resuming session: {}", sid);
            existing.paused.store(false, std::sync::atomic::Ordering::Relaxed);
            {
                let mut sender = existing.sender.lock().await;
                *sender = tx.clone();
            }
            state.sessions.insert(sid.clone(), existing.clone());
            (existing, true)
        } else {
            let session_id = sid.clone();
            let session = create_session(session_id.clone(), user_id, tx.clone());
            state.sessions.insert(session_id, session.clone());
            (session, false)
        }
    } else {
        let session_id = uuid::Uuid::new_v4().to_string();
        let session = create_session(session_id.clone(), user_id, tx.clone());
        state.sessions.insert(session_id, session.clone());
        (session, false)
    };

    let session_id = session.session_id.clone();
    info!("WebSocket connected: session={} resumed={}", session_id, resumed);

    // Send Ready
    let ready = types::OutgoingMessage::Ready {
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
            let update = types::OutgoingMessage::PlayerUpdate {
                guild_id: player.guild_id.clone(),
                state: PlayerState {
                    time: now_ms(),
                    position: player.track_handle.as_ref().map(|h| h.get_position()).unwrap_or(player.position),
                    connected: !player.voice.token.is_empty(),
                    ping: -1,
                },
            };
            if let Ok(json) = serde_json::to_string(&update) {
                let _ = socket.send(Message::Text(json.into())).await;
            }
        }
    }

    // Stats heartbeat
    let session_for_stats = session.clone();
    let state_for_stats = state.clone();
    tokio::spawn(async move {
        let start = std::time::Instant::now();
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            if session_for_stats.paused.load(std::sync::atomic::Ordering::Relaxed) {
                continue;
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
                    None => break,
                };

                match msg {
                    Message::Text(text) => {
                        match serde_json::from_str::<IncomingMessage>(&text) {
                            Ok(op) => {
                                let _ = handle_op(op, &state, &session_id).await;
                            }
                            Err(e) => {
                                warn!("Bad WS msg: session={} err={}", session_id, e);
                            }
                        }
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
        }
    }

    // Cleanup or pause for resume
    if session.resumable.load(std::sync::atomic::Ordering::Relaxed) {
        session.paused.store(true, std::sync::atomic::Ordering::Relaxed);
        state.sessions.remove(&session_id);
        state.resumable_sessions.insert(session_id.clone(), session.clone());

        let timeout_secs = session.resume_timeout.load(std::sync::atomic::Ordering::Relaxed);
        let state_cleanup = state.clone();
        let sid = session_id.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(timeout_secs)).await;
            if state_cleanup.resumable_sessions.remove(&sid).is_some() {
                info!("Session resume timeout expired: {}", sid);
            }
        });
    } else {
        state.sessions.remove(&session_id);
    }
}

fn create_session(session_id: String, user_id: Option<UserId>, tx: flume::Sender<Message>) -> Arc<Session> {
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
