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
};

pub mod handler;
pub mod heartbeat;
pub mod types;
pub mod voice;

use self::types::{
    SessionOutcome, VoiceGatewayMessage, is_fatal_close, is_reconnectable_close,
    is_reidentify_close, map_boxed_err,
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
    event_tx: Option<tokio::sync::mpsc::UnboundedSender<crate::api::LavalinkEvent>>,
    frames_sent: Arc<std::sync::atomic::AtomicU64>,
    frames_nulled: Arc<std::sync::atomic::AtomicU64>,
    cancel_token: CancellationToken,
}

const MAX_RECONNECT_ATTEMPTS: u32 = 5;

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
        event_tx: Option<tokio::sync::mpsc::UnboundedSender<crate::api::LavalinkEvent>>,
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
        let mut attempt = 0u32;
        let mut is_resume = false;
        let seq_ack = Arc::new(AtomicI64::new(-1));

        loop {
            let outcome = self.connect(is_resume, seq_ack.clone()).await;

            match outcome {
                Ok(SessionOutcome::Shutdown) => {
                    debug!(
                        "Voice gateway shutting down cleanly for guild {}",
                        self.guild_id
                    );
                    return Ok(());
                }
                Ok(SessionOutcome::Reconnect) => {
                    attempt += 1;
                    if attempt > MAX_RECONNECT_ATTEMPTS {
                        warn!(
                            "Voice gateway: max reconnect attempts ({}) reached for guild {}",
                            MAX_RECONNECT_ATTEMPTS, self.guild_id
                        );
                        return Ok(());
                    }
                    let backoff =
                        std::time::Duration::from_millis(1000 * 2u64.pow((attempt - 1).min(3)));
                    debug!(
                        "Voice gateway reconnecting (attempt {}/{}) in {:?} for guild {}",
                        attempt, MAX_RECONNECT_ATTEMPTS, backoff, self.guild_id
                    );
                    tokio::time::sleep(backoff).await;
                    is_resume = true;
                }
                Ok(SessionOutcome::Identify) => {
                    attempt += 1;
                    if attempt > MAX_RECONNECT_ATTEMPTS {
                        warn!(
                            "Voice gateway: max re-identify attempts ({}) reached for guild {}",
                            MAX_RECONNECT_ATTEMPTS, self.guild_id
                        );
                        return Ok(());
                    }
                    is_resume = false;
                    seq_ack.store(-1, Ordering::Relaxed);
                    debug!(
                        "Voice gateway session invalid; identifying fresh for guild {}",
                        self.guild_id
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
                Err(e) => {
                    attempt += 1;
                    if attempt > MAX_RECONNECT_ATTEMPTS {
                        error!(
                            "Voice gateway: connection error after {} attempts for guild {}: {}",
                            MAX_RECONNECT_ATTEMPTS, self.guild_id, e
                        );
                        return Err(e);
                    }
                    let backoff =
                        std::time::Duration::from_millis(1000 * 2u64.pow((attempt - 1).min(3)));
                    warn!(
                        "Voice gateway connection error (attempt {}/{}): {}. Retrying in {:?}",
                        attempt, MAX_RECONNECT_ATTEMPTS, e, backoff
                    );
                    tokio::time::sleep(backoff).await;
                    is_resume = false;
                }
            }
        }
    }

    async fn connect(&self, is_resume: bool, seq_ack: Arc<AtomicI64>) -> AnyResult<SessionOutcome> {
        let url = format!("wss://{}/?v=8", self.endpoint);
        debug!("Connecting to voice gateway: {}", url);

        let (ws_stream, _) = tokio_tungstenite::connect_async(&url)
            .await
            .map_err(map_boxed_err)?;
        let (mut write, mut read) = ws_stream.split();

        if is_resume {
            let resume = VoiceGatewayMessage {
                op: 7,
                d: serde_json::json!({
                    "server_id": self.guild_id,
                    "session_id": self.session_id,
                    "token": self.token,
                    "seq_ack": seq_ack.load(Ordering::Relaxed),
                }),
            };
            write
                .send(Message::Text(
                    serde_json::to_string(&resume)
                        .map_err(map_boxed_err)?
                        .into(),
                ))
                .await
                .map_err(map_boxed_err)?;
        } else {
            let identify = VoiceGatewayMessage {
                op: 0,
                d: serde_json::json!({
                    "server_id": self.guild_id,
                    "user_id": self.user_id.to_string(),
                    "session_id": self.session_id,
                    "token": self.token,
                    "max_dave_protocol_version": if self.channel_id.0 > 0 { 1 } else { 0 },
                }),
            };
            write
                .send(Message::Text(
                    serde_json::to_string(&identify)
                        .map_err(map_boxed_err)?
                        .into(),
                ))
                .await
                .map_err(map_boxed_err)?;
        }

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Message>();
        let write_task = tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                if let Err(e) = write.send(msg).await {
                    warn!(
                        "Voice WebSocket write error (expected during reconnection): {}",
                        e
                    );
                    break;
                }
            }
        });

        let mut state = handler::SessionState::new(self, tx.clone(), seq_ack.clone());

        let outcome = loop {
            let msg = match read.next().await {
                Some(Ok(msg)) => msg,
                Some(Err(e)) => {
                    warn!(
                        "Voice WebSocket read error: {}. Attempting to reconnect.",
                        e
                    );
                    if let Some(evt_tx) = &self.event_tx {
                        let _ = evt_tx.send(crate::api::LavalinkEvent::WebSocketClosed {
                            guild_id: self.guild_id.clone(),
                            code: 1006,
                            reason: format!("IO error: {}", e),
                            by_remote: true,
                        });
                    }
                    break SessionOutcome::Reconnect;
                }
                None => {
                    debug!("Voice WS stream ended for guild {}", self.guild_id);
                    if let Some(evt_tx) = &self.event_tx {
                        let _ = evt_tx.send(crate::api::LavalinkEvent::WebSocketClosed {
                            guild_id: self.guild_id.clone(),
                            code: 1000,
                            reason: "Stream ended".to_string(),
                            by_remote: true,
                        });
                    }
                    break SessionOutcome::Reconnect;
                }
            };

            match msg {
                Message::Text(text) => {
                    if let Some(outcome) = state.handle_text(text.to_string()).await {
                        break outcome;
                    }
                }
                Message::Binary(bin) => {
                    state.handle_binary(bin.to_vec()).await;
                }
                Message::Close(frame) => {
                    let (code, reason) = match &frame {
                        Some(cf) => (cf.code.into(), cf.reason.to_string()),
                        None => (1000u16, "No reason".to_string()),
                    };
                    info!(
                        "Voice WS closed: code={}, reason='{}' for guild {}",
                        code, reason, self.guild_id
                    );

                    if let Some(event_tx) = &self.event_tx {
                        let _ = event_tx.send(crate::api::LavalinkEvent::WebSocketClosed {
                            guild_id: self.guild_id.clone(),
                            code,
                            reason: reason.clone(),
                            by_remote: true,
                        });
                    }

                    if is_reconnectable_close(code) {
                        break SessionOutcome::Reconnect;
                    }
                    if is_reidentify_close(code) {
                        break SessionOutcome::Identify;
                    }
                    if is_fatal_close(code) {
                        warn!("Voice gateway closed fatally with code {}", code);
                        break SessionOutcome::Shutdown;
                    }
                    break SessionOutcome::Reconnect;
                }
                _ => {}
            }
        };

        self.cancel_token.cancel();
        drop(tx);
        let _ = tokio::time::timeout(std::time::Duration::from_millis(500), write_task).await;

        Ok(outcome)
    }
}
