use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering::Relaxed},
};

use axum::extract::ws::{CloseFrame, Message, WebSocket, close_code};
use futures::{SinkExt, StreamExt};
use tokio::sync::mpsc::error::TrySendError;
use tracing::{debug, info, trace, warn};

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

pub async fn handle_socket(
    mut socket: WebSocket,
    state: Arc<AppState>,
    user_id: Option<UserId>,
    client_session_id: Option<SessionId>,
) {
    let (tx, rx) = flume::unbounded();

    let (session, resumed) =
        resolve_session(&state, user_id, client_session_id.as_ref(), tx.clone());
    let session_id = session.session_id.clone();

    info!("WebSocket connected: session={session_id} resumed={resumed}");

    send_initial_state(&mut socket, &session, resumed).await;

    let mut stats_interval = tokio::time::interval(std::time::Duration::from_secs(
        state.config.server.stats_interval,
    ));
    stats_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let ping_interval_secs = state.config.server.websocket_ping_interval;
    let mut ping_interval =
        tokio::time::interval(std::time::Duration::from_secs(ping_interval_secs));
    ping_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let last_pong_ms = Arc::new(AtomicU64::new(0));
    let watchdog_armed = Arc::new(AtomicBool::new(false));

    let (mut ws_sink, mut ws_stream) = socket.split();
    let (ws_tx, mut ws_rx) = tokio::sync::mpsc::channel::<Message>(1024);

    let writer_session_id = session_id.clone();
    let writer_handle = tokio::spawn(async move {
        while let Some(msg) = ws_rx.recv().await {
            if let Err(e) = ws_sink.send(msg).await {
                debug!("WebSocket writer task terminating for {writer_session_id}: {e}");
                break;
            }
        }
    });

    loop {
        tokio::select! {
            _ = ping_interval.tick() => {
                if watchdog_armed.load(Relaxed) {
                    let elapsed_ms = now_ms().saturating_sub(last_pong_ms.load(Relaxed));
                    if elapsed_ms > ping_interval_secs * 3 * 1000 {
                        warn!("Client pong timeout ({elapsed_ms}ms elapsed), closing: session={session_id}");
                        break;
                    }
                }

                match ws_tx.try_send(Message::Ping(b"heartbeat".to_vec().into())) {
                    Ok(()) => {
                        if !watchdog_armed.load(Relaxed) {
                            last_pong_ms.store(now_ms(), Relaxed);
                            watchdog_armed.store(true, Relaxed);
                        }
                    }
                    Err(TrySendError::Closed(_)) => break,
                    Err(TrySendError::Full(_)) => warn!("WS ping dropped (channel full): session={session_id}"),
                }
            }
            _ = stats_interval.tick() => {
                if !session.paused.load(Relaxed) {
                    let stats = collect_stats(&state, Some(&session));
                    let msg = protocol::OutgoingMessage::Stats { stats };
                    if let Ok(json) = serde_json::to_string(&msg)
                        && let Err(e) = ws_tx.try_send(Message::Text(json.into()))
                    {
                        match e {
                            TrySendError::Closed(_) => break,
                            TrySendError::Full(_) => warn!("WS stats dropped (channel full): session={session_id}"),
                        }
                    }
                }
            }
            res = rx.recv_async() => {
                let Ok(msg) = res else {
                    warn!("WebSocket session dropped (internal channel closed): session={session_id}");
                    break;
                };
                if ws_tx.send(msg).await.is_err() {
                    break;
                }
            }
            msg = ws_stream.next() => {
                let Some(msg_result) = msg else {
                    info!("WebSocket connection closed by client: session={session_id}");
                    break;
                };

                let msg = match msg_result {
                    Ok(m) => m,
                    Err(e) => {
                        let err_msg = e.to_string();
                        if err_msg.contains("Connection reset")
                            || err_msg.contains("Broken pipe")
                            || err_msg.contains("close_notify")
                        {
                            debug!("WebSocket connection closed abruptly by client: session={session_id} err={e}");
                        } else {
                            warn!("WebSocket error from client: session={session_id} err={e}");
                        }
                        break;
                    }
                };

                match msg {
                    Message::Text(text) => {
                        match serde_json::from_str::<protocol::opcodes::IncomingMessage>(&text) {
                            Ok(op) => {
                                if let Err(e) = protocol::opcodes::handle_op(op, &state, &session_id).await {
                                    warn!("Op handling error: session={session_id} err={e}");
                                }
                            }
                            Err(e) => warn!("Failed to parse WS message: session={session_id} err={e} msg={text}"),
                        }
                    }
                    Message::Ping(payload) => {
                        if let Err(e) = ws_tx.try_send(Message::Pong(payload)) {
                            match e {
                                TrySendError::Closed(_) => break,
                                TrySendError::Full(_) => warn!("WS pong dropped (channel full): session={session_id}"),
                            }
                        }
                    }
                    Message::Pong(_) => {
                        last_pong_ms.store(now_ms(), Relaxed);
                        trace!("Heartbeat pong received: session={session_id}");
                    }
                    Message::Close(_) => {
                        info!("WebSocket received close frame: session={session_id}");
                        break;
                    }
                    _ => {}
                }
            }
        }
    }

    let _ = ws_tx.try_send(Message::Close(Some(CloseFrame {
        code: close_code::NORMAL,
        reason: "session closed".into(),
    })));

    drop(ws_tx);
    let _ = tokio::time::timeout(std::time::Duration::from_millis(500), writer_handle).await;

    handle_session_close(&state, session, &tx).await;
}

fn resolve_session(
    state: &Arc<AppState>,
    user_id: Option<UserId>,
    client_session_id: Option<&SessionId>,
    tx: flume::Sender<Message>,
) -> (Arc<Session>, bool) {
    if let Some(sid) = client_session_id
        && let Some((_, existing)) = state.resumable_sessions.remove(sid)
    {
        info!("Resuming session: {sid}");
        existing.paused.store(false, Relaxed);
        *existing.sender.lock() = tx;
        state.sessions.insert(sid.clone(), existing.clone());
        return (existing, true);
    }

    let session_id = SessionId::generate();
    let session = Arc::new(Session::new(
        session_id.clone(),
        user_id,
        tx,
        state.config.server.max_event_queue_size,
    ));
    state.sessions.insert(session_id, session.clone());

    (session, false)
}

async fn send_initial_state(socket: &mut WebSocket, session: &Arc<Session>, resumed: bool) {
    let ready = protocol::OutgoingMessage::Ready {
        resumed,
        session_id: session.session_id.clone(),
    };

    if let Ok(json) = serde_json::to_string(&ready) {
        let _ = socket.send(Message::Text(json.into())).await;
    }

    if resumed {
        let queued = std::mem::take(&mut *session.event_queue.lock());
        for json in queued {
            let _ = socket.send(Message::Text(json.into())).await;
        }

        let player_arcs: Vec<_> = session
            .players
            .iter()
            .map(|kv| kv.value().clone())
            .collect();
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
                    ping: player.ping.load(Relaxed),
                },
            };
            session.send_message(&update);
        }
    }
}

async fn handle_session_close(
    state: &Arc<AppState>,
    session: Arc<Session>,
    tx: &flume::Sender<Message>,
) {
    let session_id = session.session_id.clone();

    if session.resumable.load(Relaxed) {
        session.paused.store(true, Relaxed);

        if !session.sender.lock().same_channel(tx) {
            info!(
                "Session {session_id} replaced by a new connection; closing the old connection for cleanup."
            );
            return;
        }

        state.sessions.remove(&session_id);

        if let Some((_, removed)) = state.resumable_sessions.remove(&session_id) {
            warn!(
                "Shutting down resumable session {} because it shares an ID with a newly disconnected session.",
                removed.session_id
            );
            removed.shutdown(state).await;
        }

        state
            .resumable_sessions
            .insert(session_id.clone(), session.clone());

        let timeout_secs = session.resume_timeout.load(Relaxed);
        info!(
            "Connection closed (resumable). Session {session_id} can be resumed within {timeout_secs} seconds."
        );

        let state_cleanup = state.clone();
        let sid = session_id.clone();

        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(timeout_secs)).await;
            if let Some((_, s)) = state_cleanup.resumable_sessions.remove(&sid) {
                warn!("Session resume timeout expired: {sid}");
                s.shutdown(&state_cleanup).await;
            }
        });
    } else if let Some((_, s)) = state.sessions.remove(&session_id) {
        info!("Connection closed (not resumable): {session_id}");
        s.shutdown(state).await;
    }
}
