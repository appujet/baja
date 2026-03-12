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
    gateway::constants::{VOICE_GATEWAY_VERSION, WRITE_TASK_SHUTDOWN_MS},
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
    /// Creates a new VoiceGateway from the provided configuration.
    ///
    /// The returned gateway copies identifying information and shared resources from
    /// `config` and prepares runtime primitives: an empty UDP socket slot, a
    /// `DaveHandler` for the configured user and channel, a new outer
    /// `CancellationToken`, and a `FailurePolicy` initialized for up to 3 attempts.
    ///
    /// # Parameters
    ///
    /// - `config`: configuration payload containing guild/user/channel identifiers,
    ///   authentication token, endpoint, audio mixer and filter chain, ping state,
    ///   optional event sender, and frame counters.
    ///
    /// # Returns
    ///
    /// A `VoiceGateway` instance configured and ready to be started via `run`.
    ///
    /// # Examples
    ///
    /// ```
    /// // given a properly constructed `config: VoiceGatewayConfig`
    /// let gw = VoiceGateway::new(config);
    /// // the gateway preserves the config's identifiers
    /// // assert_eq!(gw.guild_id, config.guild_id);
    /// ```
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

    /// Runs the voice gateway connection loop until shutdown, cancellation, or an unrecoverable error.
    ///
    /// The method repeatedly attempts to establish and maintain a voice gateway session, applying
    /// the gateway's failure policy and backoff strategy. It will attempt session resume when
    /// appropriate and reset session state on fresh reconnects.
    ///
    /// # Returns
    ///
    /// `Ok(())` when the run loop exits normally (shutdown or cancellation), `Err(GatewayError)` on an unrecoverable connection failure.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use std::sync::Arc;
    /// # use tokio::sync::Mutex;
    /// # use rustalink::gateway::session::VoiceGateway; // placeholder path
    /// # async fn example(mut gw: VoiceGateway) -> Result<(), rustalink::GatewayError> {
    /// gw.run().await
    /// # }
    /// ```
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

    /// Establishes a WebSocket connection to the voice gateway and drives the session until it yields a session outcome.
    ///
    /// This attempts the handshake (identify or resume), sets up read/write tasks and a per-connection cancellation token,
    /// forwards outgoing messages, handles incoming gateway frames via `handle_message`, and returns the final `SessionOutcome`.
    ///
    /// # Parameters
    ///
    /// - `is_resume`: If `true`, sends a resume handshake; otherwise sends an identify handshake.
    /// - `seq_ack`: Atomic sequence acknowledgement used to populate resume sequence and to track last acked sequence.
    /// - `persistent_state`: Shared session state persisted across reconnect attempts.
    /// - `backoff`: Mutable backoff state used/updated by the session state during connection setup.
    ///
    /// # Returns
    ///
    /// `SessionOutcome` indicating why the session ended (e.g., `Shutdown` or `Reconnect`), or a `GatewayError` on connection/setup failure.
    ///
    /// # Examples
    ///
    /// ```
    /// # use std::sync::Arc;
    /// # use tokio::sync::Mutex;
    /// # use std::sync::atomic::AtomicI64;
    /// # use rustalink::gateway::session::{VoiceGateway, VoiceGatewayConfig};
    /// # use rustalink::gateway::session::types::PersistentSessionState;
    /// # use rustalink::gateway::session::Backoff;
    /// #
    /// # #[tokio::test]
    /// # async fn connects_and_returns_outcome() {
    /// // Construct a VoiceGateway (fields elided for brevity) and supporting state.
    /// let config = VoiceGatewayConfig { /* fields omitted */ };
    /// let gateway = VoiceGateway::new(config);
    /// let seq_ack = Arc::new(AtomicI64::new(-1));
    /// let persistent = Arc::new(Mutex::new(PersistentSessionState::default()));
    /// let mut backoff = Backoff::new();
    ///
    /// // Attempt a fresh connection (not a resume).
    /// let outcome = gateway.connect(false, seq_ack, persistent, &mut backoff).await;
    /// match outcome {
    ///     Ok(_) => { /* connection produced an outcome */ }
    ///     Err(_) => { /* connection failed */ }
    /// }
    /// # }
    /// ```
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

        tokio::spawn(async move {
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
        if let Some(Ok(m)) = read.next().await
            && let Some(out) = self.handle_message(&mut state, m).await
        {
            conn_token.cancel();
            return Ok(out);
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
        let _ = tokio::time::timeout(
            std::time::Duration::from_millis(WRITE_TASK_SHUTDOWN_MS),
            tokio::task::yield_now(),
        )
        .await;

        Ok(outcome)
    }

    /// Handle an incoming WebSocket `Message` for the current session and produce an optional
    /// session outcome when the message requires a session-level decision.
    ///
    /// This inspects the provided `msg` and delegates processing to the session `state` for text
    /// and binary payloads, replies to pings with a pong, and classifies/records close frames
    /// via the gateway's failure `policy`.
    ///
    /// # Returns
    ///
    /// `Some(SessionOutcome)` when the message indicates the session should transition (for example,
    /// on a close frame); `None` for messages that were consumed without requiring a session outcome.
    ///
    /// # Examples
    ///
    /// ```
    /// # use tokio::runtime::Runtime;
    /// # use tokio_tungstenite::tungstenite::Message;
    /// # // The following is a conceptual example; constructing a real `VoiceGateway` and
    /// # // `handler::SessionState` requires the surrounding module context.
    /// # let rt = Runtime::new().unwrap();
    /// # rt.block_on(async {
    /// // let gateway: VoiceGateway = ...;
    /// // let mut state: handler::SessionState<'_> = ...;
    /// // Example: a ping should be replied to and yield no session outcome.
    /// // let outcome = gateway.handle_message(&mut state, Message::Ping(vec![])).await;
    /// // assert!(outcome.is_none());
    /// # });
    /// ```
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

    /// Sends a "speaking" gateway payload over the provided WebSocket send channel.
    ///
    /// The payload sets `speaking` to `1` when `speaking` is true, otherwise `0`, includes a
    /// `delay` of `0`, and the provided `ssrc`. If JSON serialization fails the send is skipped.
    ///
    /// # Examples
    ///
    /// ```
    /// use futures::executor::block_on;
    /// use futures::StreamExt;
    /// use futures::channel::mpsc::unbounded;
    /// use tokio_tungstenite::tungstenite::Message;
    ///
    /// struct Dummy;
    /// impl Dummy {
    ///     fn notify_speaking(&self, tx: &futures::channel::mpsc::UnboundedSender<Message>, ssrc: u32, speaking: bool) {
    ///         let msg = serde_json::json!({
    ///             "op": 5u8, // example op code placeholder
    ///             "seq": null,
    ///             "d": {
    ///                 "speaking": if speaking { 1 } else { 0 },
    ///                 "delay": 0,
    ///                 "ssrc": ssrc
    ///             }
    ///         });
    ///         if let Ok(json) = serde_json::to_string(&msg) {
    ///             let _ = tx.unbounded_send(Message::Text(json));
    ///         }
    ///     }
    /// }
    ///
    /// let gw = Dummy;
    /// block_on(async {
    ///     let (tx, mut rx) = unbounded::<Message>();
    ///     gw.notify_speaking(&tx, 42, true);
    ///     let msg = rx.next().await.unwrap();
    ///     if let Message::Text(s) = msg {
    ///         assert!(s.contains(r#""speaking":1"#));
    ///         assert!(s.contains(r#""ssrc":42"#));
    ///     } else {
    ///         panic!("expected text message");
    ///     }
    /// });
    /// ```
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

    /// Emits a WebSocket-closed event to the optional event channel with the provided close code and reason.
    ///
    /// If an event channel is configured, sends a `RustalinkEvent::WebSocketClosed` containing this gateway's
    /// guild identifier, the numeric close `code`, the textual `reason`, and `by_remote = true`.
    ///
    /// # Examples
    ///
    /// ```
    /// // Assuming `gw` is a `VoiceGateway` with an event channel configured:
    /// // gw.emit_close(4000, "session expired".into());
    /// ```
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
