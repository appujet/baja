use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};

use futures::{SinkExt, StreamExt};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use tokio_tungstenite::tungstenite::{
    Error as WsError,
    error::ProtocolError,
    protocol::{Message, WebSocketConfig},
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, trace, warn};

use crate::{
    audio::{Mixer, filters::FilterChain},
    common::types::{AnyResult, ChannelId, GuildId, SessionId, Shared, UserId},
    gateway::constants::{RECONNECT_DELAY_FRESH_MS, VOICE_GATEWAY_VERSION, WRITE_TASK_SHUTDOWN_MS},
    protocol::RustalinkEvent,
};

pub mod backoff;
pub mod handler;
pub mod heartbeat;
pub mod types;
pub mod voice;

use self::{
    backoff::Backoff,
    types::{
        PersistentSessionState, SessionOutcome, VoiceGatewayMessage, classify_close, map_boxed_err,
    },
};

pub struct VoiceGateway {
    pub guild_id: GuildId,
    pub user_id: UserId,
    pub channel_id: ChannelId,
    session_id: SessionId,
    token: String,
    endpoint: String,
    pub mixer: Shared<Mixer>,
    pub filter_chain: Shared<FilterChain>,
    pub ping: Arc<AtomicI64>,
    event_tx: Option<UnboundedSender<RustalinkEvent>>,
    pub frames_sent: Arc<std::sync::atomic::AtomicU64>,
    pub frames_nulled: Arc<std::sync::atomic::AtomicU64>,
    outer_token: CancellationToken,
}

impl Drop for VoiceGateway {
    fn drop(&mut self) {
        self.outer_token.cancel();
    }
}

pub struct VoiceGatewayConfig {
    pub guild_id: GuildId,
    pub user_id: UserId,
    pub channel_id: ChannelId,
    pub session_id: SessionId,
    pub token: String,
    pub endpoint: String,
    pub mixer: Shared<Mixer>,
    pub filter_chain: Shared<FilterChain>,
    pub ping: Arc<AtomicI64>,
    pub event_tx: Option<UnboundedSender<RustalinkEvent>>,
    pub frames_sent: Arc<std::sync::atomic::AtomicU64>,
    pub frames_nulled: Arc<std::sync::atomic::AtomicU64>,
}

impl VoiceGateway {
    pub fn new(config: VoiceGatewayConfig) -> Self {
        Self {
            guild_id: config.guild_id,
            user_id: config.user_id,
            channel_id: config.channel_id,
            session_id: config.session_id,
            token: config.token,
            endpoint: config.endpoint,
            mixer: config.mixer,
            filter_chain: config.filter_chain,
            ping: config.ping,
            event_tx: config.event_tx,
            frames_sent: config.frames_sent,
            frames_nulled: config.frames_nulled,
            outer_token: CancellationToken::new(),
        }
    }

    pub async fn run(self) -> AnyResult<()> {
        let mut backoff = Backoff::new();
        let mut is_resume = false;
        let seq_ack = Arc::new(AtomicI64::new(-1));
        let persistent_state = Arc::new(tokio::sync::Mutex::new(PersistentSessionState::default()));

        while !self.outer_token.is_cancelled() {
            match self
                .connect(
                    is_resume,
                    seq_ack.clone(),
                    persistent_state.clone(),
                    &mut backoff,
                )
                .await
            {
                Ok(SessionOutcome::Shutdown) => {
                    debug!("[{}] Gateway shutting down cleanly", self.guild_id);
                    return Ok(());
                }
                Ok(SessionOutcome::Reconnect) => {
                    if backoff.is_exhausted() {
                        warn!("[{}] Max reconnect attempts reached", self.guild_id);
                        self.emit_close_event(1006, "Max reconnect attempts reached".into());
                        return Ok(());
                    }
                    let delay = backoff.next();
                    debug!(
                        "[{}] Reconnecting in {:?} (resume=true)",
                        self.guild_id, delay
                    );
                    tokio::time::sleep(delay).await;
                    is_resume = true;
                }
                Ok(SessionOutcome::Identify) => {
                    if backoff.is_exhausted() {
                        warn!("[{}] Max re-identify attempts reached", self.guild_id);
                        self.emit_close_event(1006, "Max re-identify attempts reached".into());
                        return Ok(());
                    }
                    is_resume = false;
                    seq_ack.store(-1, Ordering::Relaxed);
                    // Clear persistent state on identify to avoid using stale keys/addr
                    {
                        let mut state = persistent_state.lock().await;
                        state.udp_addr = None;
                        state.session_key = None;
                    }
                    let delay = std::time::Duration::from_millis(RECONNECT_DELAY_FRESH_MS);
                    debug!(
                        "[{}] Session invalid; identifying fresh in {:?}",
                        self.guild_id, delay
                    );
                    tokio::time::sleep(delay).await;
                    // Note: Identify backoff is handled differently in reference codes,
                    // but we'll stick to our backoff logic for now and just increment it.
                    backoff.next();
                }
                Err(e) => {
                    if backoff.is_exhausted() {
                        error!("[{}] Connection failed: {e}", self.guild_id);
                        self.emit_close_event(1006, format!("Connection failed: {e}"));
                        return Ok(());
                    }
                    let delay = backoff.next();
                    warn!(
                        "[{}] Connection error: {e}. Retrying in {:?}",
                        self.guild_id, delay
                    );
                    tokio::time::sleep(delay).await;
                    is_resume = false;
                }
            }
        }
        Ok(())
    }

    async fn connect(
        &self,
        is_resume: bool,
        seq_ack: Arc<AtomicI64>,
        persistent_state: Arc<tokio::sync::Mutex<PersistentSessionState>>,
        backoff: &mut Backoff,
    ) -> AnyResult<SessionOutcome> {
        let mut endpoint = self.endpoint.clone();
        if endpoint.ends_with(":80") {
            endpoint.truncate(endpoint.len() - 3);
        }

        let url = format!("wss://{}/?v={}", endpoint, VOICE_GATEWAY_VERSION);
        debug!(
            "[{}] Connecting to voice gateway: {url} (attempt {})",
            self.guild_id,
            backoff.attempt()
        );

        let mut config = WebSocketConfig::default();
        config.max_message_size = None;
        config.max_frame_size = None;

        let (ws_stream, _) = tokio_tungstenite::connect_async_with_config(&url, Some(config), true)
            .await
            .map_err(map_boxed_err)?;
        let (mut write, mut read) = ws_stream.split();

        let handshake = if is_resume {
            trace!(
                "[{}] Sending voice RESUME: {:?}",
                self.guild_id, self.session_id
            );
            self.resume_message(seq_ack.load(Ordering::Relaxed))
        } else {
            trace!(
                "[{}] Sending voice IDENTIFY: {:?}",
                self.guild_id, self.session_id
            );
            self.identify_message()
        };

        write
            .send(Message::Text(
                serde_json::to_string(&handshake)
                    .map_err(map_boxed_err)?
                    .into(),
            ))
            .await
            .map_err(map_boxed_err)?;

        let conn_token = CancellationToken::new();
        let (ws_tx, mut ws_rx) = unbounded_channel::<Message>();

        let guild_id = self.guild_id.clone();
        let write_token = conn_token.clone();
        tokio::spawn(async move {
            while let Some(msg) = tokio::select! {
                biased;
                _ = write_token.cancelled() => None,
                msg = ws_rx.recv() => msg,
            } {
                if let Err(e) = write.send(msg).await {
                    warn!("[{}] WS write error: {e}", guild_id);
                    break;
                }
            }
        });

        let (speaking_tx, mut speaking_rx) = unbounded_channel::<bool>();

        let mut state = handler::SessionState::new(
            self,
            ws_tx.clone(),
            seq_ack.clone(),
            conn_token.clone(),
            speaking_tx,
            persistent_state,
            backoff,
        )
        .map_err(|e| {
            warn!("[{}] Init session failed: {e}", self.guild_id);
            conn_token.cancel();
            e
        })?;

        let outcome = loop {
            tokio::select! {
                biased;
                _ = self.outer_token.cancelled() => break SessionOutcome::Shutdown,
                _ = conn_token.cancelled() => {
                    warn!("[{}] Connection token cancelled (heartbeat timeout?)", self.guild_id);
                    break SessionOutcome::Reconnect;
                }
                Some(is_speaking) = speaking_rx.recv() => {
                    self.send_speaking_notification(&ws_tx, state.ssrc(), is_speaking);
                }
                msg = read.next() => {
                    match msg {
                        Some(Ok(m)) => if let Some(out) = self.handle_ws_message(&mut state, m).await {
                            break out;
                        },
                        Some(Err(e)) => {
                            let is_reset = matches!(
                                e,
                                WsError::Protocol(ProtocolError::ResetWithoutClosingHandshake)
                            );

                            let is_tls_eof = matches!(&e, WsError::Io(io_err)
                                if io_err.to_string().contains("close_notify"));

                            if is_reset || is_tls_eof {
                                debug!(
                                    "[{}] WS connection closed by peer without handshake (reset={is_reset} tls_eof={is_tls_eof})",
                                    self.guild_id
                                );
                            } else {
                                warn!("[{}] WS read error: {e}", self.guild_id);
                                self.emit_close_event(1006, format!("IO error: {e}"));
                            }
                            break SessionOutcome::Reconnect;
                        }
                        None => {
                            debug!("[{}] WS ended", self.guild_id);
                            self.emit_close_event(1000, "Stream ended".into());
                            break SessionOutcome::Reconnect;
                        }
                    }
                }
            }
        };

        conn_token.cancel();
        let _ = tokio::time::timeout(
            std::time::Duration::from_millis(WRITE_TASK_SHUTDOWN_MS),
            tokio::task::yield_now(),
        )
        .await;

        Ok(outcome)
    }

    fn send_speaking_notification(
        &self,
        tx: &UnboundedSender<Message>,
        ssrc: u32,
        is_speaking: bool,
    ) {
        let msg = VoiceGatewayMessage {
            op: 5,
            d: serde_json::json!({
                "speaking": if is_speaking { 1u8 } else { 0u8 },
                "delay": 0,
                "ssrc": ssrc
            }),
        };
        if let Ok(json) = serde_json::to_string(&msg) {
            let _ = tx.send(Message::Text(json.into()));
        }
    }

    async fn handle_ws_message(
        &self,
        state: &mut handler::SessionState<'_>,
        msg: Message,
    ) -> Option<SessionOutcome> {
        match msg {
            Message::Text(text) => state.handle_text(text.to_string()).await,
            Message::Binary(bin) => {
                state.handle_binary(bin.to_vec()).await;
                None
            }
            Message::Close(frame) => {
                let (code, reason) = frame
                    .map(|cf| (cf.code.into(), cf.reason.to_string()))
                    .unwrap_or((1000u16, "No reason".into()));

                info!(
                    "[{}] WS close: code={code} reason='{reason}'",
                    self.guild_id
                );
                self.emit_close_event(code, reason);
                Some(classify_close(code))
            }
            Message::Ping(payload) => {
                let _ = state.tx().send(Message::Pong(payload));
                None
            }
            _ => None,
        }
    }

    fn identify_message(&self) -> VoiceGatewayMessage {
        VoiceGatewayMessage {
            op: 0,
            d: serde_json::json!({
                "server_id": self.guild_id.to_string(),
                "user_id": self.user_id.0.to_string(),
                "session_id": self.session_id,
                "token": self.token,
                "video": true,
                "max_dave_protocol_version": if self.channel_id.0 > 0 { 1 } else { 0 },
            }),
        }
    }

    fn resume_message(&self, seq_ack: i64) -> VoiceGatewayMessage {
        VoiceGatewayMessage {
            op: 7,
            d: serde_json::json!({
                "server_id": self.guild_id.to_string(),
                "session_id": self.session_id,
                "token": self.token,
                "video": true,
                "seq_ack": seq_ack,
            }),
        }
    }

    fn emit_close_event(&self, code: u16, reason: String) {
        if let Some(tx) = &self.event_tx {
            let _ = tx.send(RustalinkEvent::WebSocketClosed {
                guild_id: self.guild_id.clone(),
                code,
                reason,
                by_remote: true,
            });
        }
    }
}
