use futures::{SinkExt, StreamExt};
use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};
use tokio_tungstenite::tungstenite::protocol::Message;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::{
    audio::Mixer,
    common::types::{AnyResult, Shared},
    gateway::constants::{RECONNECT_DELAY_FRESH_MS, VOICE_GATEWAY_VERSION, WRITE_TASK_SHUTDOWN_MS},
};

pub mod backoff;
pub mod handler;
pub mod heartbeat;
pub mod types;
pub mod voice;

use self::{
    backoff::Backoff,
    types::{
        SessionOutcome, VoiceGatewayMessage, is_fatal_close, is_reconnectable_close,
        is_reidentify_close, map_boxed_err,
    },
};

pub struct VoiceGateway {
    guild_id: crate::common::types::GuildId,
    user_id: crate::common::types::UserId,
    channel_id: crate::common::types::ChannelId,
    session_id: crate::common::types::SessionId,
    token: String,
    endpoint: String,
    mixer: Shared<Mixer>,
    filter_chain: Shared<crate::audio::filters::FilterChain>,
    ping: Arc<AtomicI64>,
    event_tx: Option<tokio::sync::mpsc::UnboundedSender<crate::protocol::RustalinkEvent>>,
    frames_sent: Arc<std::sync::atomic::AtomicU64>,
    frames_nulled: Arc<std::sync::atomic::AtomicU64>,
    cancel_token: CancellationToken,
}

impl Drop for VoiceGateway {
    fn drop(&mut self) {
        self.cancel_token.cancel();
    }
}

impl VoiceGateway {
    pub fn new(
        guild_id: crate::common::types::GuildId,
        user_id: crate::common::types::UserId,
        channel_id: crate::common::types::ChannelId,
        session_id: crate::common::types::SessionId,
        token: String,
        endpoint: String,
        mixer: Shared<Mixer>,
        filter_chain: Shared<crate::audio::filters::FilterChain>,
        ping: Arc<AtomicI64>,
        event_tx: Option<tokio::sync::mpsc::UnboundedSender<crate::protocol::RustalinkEvent>>,
        frames_sent: Arc<std::sync::atomic::AtomicU64>,
        frames_nulled: Arc<std::sync::atomic::AtomicU64>,
    ) -> Self {
        Self {
            guild_id,
            user_id,
            channel_id,
            session_id,
            token,
            endpoint,
            mixer,
            filter_chain,
            ping,
            event_tx,
            frames_sent,
            frames_nulled,
            cancel_token: CancellationToken::new(),
        }
    }

    pub async fn run(self) -> AnyResult<()> {
        let mut backoff = Backoff::new();
        let mut is_resume = false;
        let seq_ack = Arc::new(AtomicI64::new(-1));

        loop {
            if self.cancel_token.is_cancelled() {
                return Ok(());
            }

            let outcome = self.connect(is_resume, seq_ack.clone()).await;

            match outcome {
                Ok(SessionOutcome::Shutdown) => {
                    debug!("[{}] Gateway shutting down cleanly", self.guild_id);
                    return Ok(());
                }
                Ok(SessionOutcome::Reconnect) => {
                    if backoff.is_exhausted() {
                        warn!("[{}] Max reconnect attempts reached", self.guild_id);
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
                        return Ok(());
                    }
                    is_resume = false;
                    seq_ack.store(-1, Ordering::Relaxed);
                    let delay = std::time::Duration::from_millis(RECONNECT_DELAY_FRESH_MS);
                    debug!(
                        "[{}] Session invalid; identifying fresh in {:?}",
                        self.guild_id, delay
                    );
                    tokio::time::sleep(delay).await;
                    backoff.next();
                }
                Err(e) => {
                    if backoff.is_exhausted() {
                        error!(
                            "[{}] Connection error after max attempts: {}",
                            self.guild_id, e
                        );
                        return Err(e);
                    }
                    let delay = backoff.next();
                    warn!(
                        "[{}] Connection error: {}. Retrying in {:?}",
                        self.guild_id, e, delay
                    );
                    tokio::time::sleep(delay).await;
                    is_resume = false;
                }
            }
        }
    }

    async fn connect(&self, is_resume: bool, seq_ack: Arc<AtomicI64>) -> AnyResult<SessionOutcome> {
        let url = format!("wss://{}/?v={}", self.endpoint, VOICE_GATEWAY_VERSION);
        debug!("[{}] Connecting to voice gateway: {}", self.guild_id, url);

        let (ws_stream, _) = tokio_tungstenite::connect_async(&url)
            .await
            .map_err(map_boxed_err)?;
        let (mut write, mut read) = ws_stream.split();

        let msg = if is_resume {
            self.resume_message(seq_ack.load(Ordering::Relaxed))
        } else {
            self.identify_message()
        };

        let json = serde_json::to_string(&msg).map_err(map_boxed_err)?;
        write
            .send(Message::Text(json.into()))
            .await
            .map_err(map_boxed_err)?;

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Message>();

        let cancel = self.cancel_token.clone();
        let guild_id = self.guild_id.clone();
        let write_task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => break,
                    msg = rx.recv() => {
                        let Some(msg) = msg else { break };
                        if let Err(e) = write.send(msg).await {
                            warn!("[{}] WS write error: {}", guild_id, e);
                            break;
                        }
                    }
                }
            }
        });

        let mut state = handler::SessionState::new(self, tx.clone(), seq_ack.clone());

        let outcome = loop {
            tokio::select! {
                _ = self.cancel_token.cancelled() => {
                    break SessionOutcome::Shutdown;
                }
                msg = read.next() => {
                    let msg = match msg {
                        Some(Ok(msg)) => msg,
                        Some(Err(e)) => {
                            warn!("[{}] WS read error: {}", self.guild_id, e);
                            self.emit_close_event(1006, format!("IO error: {}", e));
                            break SessionOutcome::Reconnect;
                        }
                        None => {
                            debug!("[{}] WS stream ended", self.guild_id);
                            self.emit_close_event(1000, "Stream ended".into());
                            break SessionOutcome::Reconnect;
                        }
                    };

                    match msg {
                        Message::Text(text) => {
                            // Pass the owned String directly — no extra allocation.
                            if let Some(outcome) = state.handle_text(text.to_string()).await {
                                break outcome;
                            }
                        }
                        Message::Binary(bin) => {
                            // Pass the owned Bytes directly — no extra allocation.
                            state.handle_binary(bin.to_vec()).await;
                        }
                        Message::Close(frame) => {
                            let (code, reason) = frame
                                .map(|cf| (cf.code.into(), cf.reason.to_string()))
                                .unwrap_or((1000u16, "No reason".into()));

                            info!(
                                "[{}] WS closed: code={}, reason='{}'",
                                self.guild_id, code, reason
                            );
                            self.emit_close_event(code, reason.clone());

                            if is_reconnectable_close(code) {
                                break SessionOutcome::Reconnect;
                            }
                            if is_reidentify_close(code) {
                                break SessionOutcome::Identify;
                            }
                            if is_fatal_close(code) {
                                break SessionOutcome::Shutdown;
                            }
                            break SessionOutcome::Reconnect;
                        }
                        _ => {}
                    }
                }
            }
        };

        self.cancel_token.cancel();
        drop(tx);
        let _ = tokio::time::timeout(
            std::time::Duration::from_millis(WRITE_TASK_SHUTDOWN_MS),
            write_task,
        )
        .await;

        Ok(outcome)
    }

    fn identify_message(&self) -> VoiceGatewayMessage {
        VoiceGatewayMessage {
            op: 0,
            d: serde_json::json!({
                "server_id": self.guild_id,
                "user_id": self.user_id.to_string(),
                "session_id": self.session_id,
                "token": self.token,
                "max_dave_protocol_version": if self.channel_id.0 > 0 { 1 } else { 0 },
            }),
        }
    }

    fn resume_message(&self, seq_ack: i64) -> VoiceGatewayMessage {
        VoiceGatewayMessage {
            op: 7,
            d: serde_json::json!({
                "server_id": self.guild_id,
                "session_id": self.session_id,
                "token": self.token,
                "seq_ack": seq_ack,
            }),
        }
    }

    fn emit_close_event(&self, code: u16, reason: String) {
        if let Some(tx) = &self.event_tx {
            let _ = tx.send(crate::protocol::RustalinkEvent::WebSocketClosed {
                guild_id: self.guild_id.clone(),
                code,
                reason,
                by_remote: true,
            });
        }
    }
}
