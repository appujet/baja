use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};

use futures::{SinkExt, StreamExt};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use tokio_tungstenite::tungstenite::protocol::{Message, WebSocketConfig};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, warn};

use crate::{
    audio::{Mixer, filters::FilterChain},
    common::types::{ChannelId, GuildId, SessionId, Shared, UserId},
    gateway::constants::VOICE_GATEWAY_VERSION,
    protocol::RustalinkEvent,
};

pub mod backoff;
pub mod handler;
pub mod heartbeat;
pub mod policy;
pub mod protocol;
pub mod types;
pub mod voice;

use self::{
    backoff::Backoff,
    policy::FailurePolicy,
    types::{GatewayError, PersistentSessionState, SessionOutcome},
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
    pub udp_socket: Shared<Option<Arc<tokio::net::UdpSocket>>>,
    pub dave: Shared<crate::gateway::DaveHandler>,
    outer_token: CancellationToken,
    policy: FailurePolicy,
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
            udp_socket: Arc::new(tokio::sync::Mutex::new(None)),
            dave: Arc::new(tokio::sync::Mutex::new(crate::gateway::DaveHandler::new(
                config.user_id,
                config.channel_id,
            ))),
            outer_token: CancellationToken::new(),
            policy: FailurePolicy::new(3),
        }
    }

    pub async fn run(self) -> Result<(), GatewayError> {
        let mut backoff = Backoff::new();
        let mut is_resume = false;
        let seq_ack = Arc::new(AtomicI64::new(-1));
        let persistent_state = Arc::new(tokio::sync::Mutex::new(PersistentSessionState::default()));

        while !self.outer_token.is_cancelled() {
            let attempt = backoff.attempt();
            match self
                .connect(
                    is_resume,
                    seq_ack.clone(),
                    persistent_state.clone(),
                    &mut backoff,
                )
                .await
            {
                Ok(SessionOutcome::Shutdown) => break,
                Ok(outcome) => {
                    if backoff.is_exhausted() {
                        warn!("[{}] Max attempts reached ({})", self.guild_id, attempt);
                        break;
                    }

                    let delay = backoff.next_delay();
                    is_resume = matches!(outcome, SessionOutcome::Reconnect);

                    if !is_resume {
                        seq_ack.store(-1, Ordering::Relaxed);
                        *persistent_state.lock().await = PersistentSessionState::default();
                        *self.udp_socket.lock().await = None;
                    }

                    debug!(
                        "[{}] Retrying ({:?}) in {:?}",
                        self.guild_id, outcome, delay
                    );
                    tokio::time::sleep(delay).await;
                }
                Err(e) => {
                    if backoff.is_exhausted() {
                        error!("[{}] Fatal connection error: {e}", self.guild_id);
                        break;
                    }
                    let delay = backoff.next_delay();
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
    ) -> Result<SessionOutcome, GatewayError> {
        let endpoint = if self.endpoint.ends_with(":80") {
            &self.endpoint[..self.endpoint.len() - 3]
        } else {
            &self.endpoint
        };

        let url = format!("wss://{}/?v={}", endpoint, VOICE_GATEWAY_VERSION);
        let mut config = WebSocketConfig::default();
        config.max_message_size = None;
        config.max_frame_size = None;

        let (ws_stream, _) =
            tokio_tungstenite::connect_async_with_config(&url, Some(config), true).await?;

        let (mut write, mut read) = ws_stream.split();
        let conn_token = CancellationToken::new();
        let write_token = conn_token.clone();
        let (ws_tx, mut ws_rx) = unbounded_channel::<Message>();

        let writer_handle = tokio::spawn(async move {
            while let Some(msg) = tokio::select! {
                biased;
                _ = write_token.cancelled() => None,
                msg = ws_rx.recv() => msg,
            } {
                if write.send(msg).await.is_err() {
                    break;
                }
            }
        });

        let mut state = handler::SessionState::new(
            self,
            ws_tx.clone(),
            seq_ack.clone(),
            conn_token.clone(),
            persistent_state,
            backoff,
        )
        .await
        .inspect_err(|_e| {
            conn_token.cancel();
        })?;

        // Wait for Op 8 HELLO
        let outcome = match read.next().await {
            Some(Ok(m)) => self.handle_message(&mut state, m).await,
            _ => Some(SessionOutcome::Reconnect),
        };

        if let Some(out) = outcome {
            conn_token.cancel();
            writer_handle.abort();
            let _ = writer_handle.await;
            return Ok(out);
        }

        if !state.has_heartbeat() {
            conn_token.cancel();
            writer_handle.abort();
            let _ = writer_handle.await;
            return Ok(SessionOutcome::Reconnect);
        }

        let handshake = if is_resume {
            protocol::builders::resume(
                self.guild_id.to_string(),
                self.session_id.to_string(),
                self.token.clone(),
                seq_ack.load(Ordering::Relaxed),
            )
        } else {
            protocol::builders::identify(
                self.guild_id.to_string(),
                self.user_id.0.to_string(),
                self.session_id.to_string(),
                self.token.clone(),
                1,
            )
        };

        let _ = ws_tx.send(Message::Text(
            serde_json::to_string(&handshake).unwrap().into(),
        ));

        let (speaking_tx, mut speaking_rx) = unbounded_channel::<bool>();
        state.set_speaking_tx(speaking_tx);

        let outcome = loop {
            tokio::select! {
                biased;
                _ = self.outer_token.cancelled() => break SessionOutcome::Shutdown,
                _ = conn_token.cancelled() => break SessionOutcome::Reconnect,
                Some(speaking) = speaking_rx.recv() => {
                    self.notify_speaking(&ws_tx, state.ssrc(), speaking);
                }
                msg = read.next() => match msg {
                    Some(Ok(m)) => if let Some(out) = self.handle_message(&mut state, m).await {
                        break out;
                    },
                    Some(Err(_)) => break SessionOutcome::Reconnect,
                    None => break SessionOutcome::Reconnect,
                }
            }
        };

        conn_token.cancel();
        writer_handle.abort();
        let _ = writer_handle.await;

        Ok(outcome)
    }

    async fn handle_message(
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
                let code = frame.as_ref().map(|f| f.code.into()).unwrap_or(1000u16);
                let reason = frame.map(|f| f.reason.to_string()).unwrap_or_default();
                let attempt = state.attempt();

                debug!("[{}] Gateway closed: {} ({})", self.guild_id, code, reason);

                if !self.policy.is_retryable(code, attempt) {
                    self.emit_close(code, reason);
                }

                Some(self.policy.classify(code))
            }
            Message::Ping(p) => {
                let _ = state.tx().send(Message::Pong(p));
                None
            }
            _ => None,
        }
    }

    fn notify_speaking(&self, tx: &UnboundedSender<Message>, ssrc: u32, speaking: bool) {
        let msg = protocol::GatewayPayload {
            op: protocol::OpCode::Speaking as u8,
            seq: None,
            d: serde_json::json!({
                "speaking": if speaking { 1 } else { 0 },
                "delay": 0,
                "ssrc": ssrc
            }),
        };
        if let Ok(json) = serde_json::to_string(&msg) {
            let _ = tx.send(Message::Text(json.into()));
        }
    }

    fn emit_close(&self, code: u16, reason: String) {
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
