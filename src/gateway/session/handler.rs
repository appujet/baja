use std::{
    collections::HashSet,
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    },
};

use serde_json::Value;
use tokio::sync::mpsc::UnboundedSender;
use tokio_tungstenite::tungstenite::protocol::Message;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, trace, warn};
use uuid::Uuid;

use super::{
    VoiceGateway,
    backoff::Backoff,
    heartbeat::HeartbeatTracker,
    protocol::{GatewayPayload, OpCode},
    types::{GatewayError, PersistentSessionState, SessionOutcome},
    voice::{SpeakConfig, discover_ip, speak_loop},
};
use crate::{
    common::types::{Shared, UserId},
    gateway::{
        DaveHandler,
        constants::{DAVE_INITIAL_VERSION, DEFAULT_VOICE_MODE},
    },
};

pub struct SessionState<'a> {
    gateway: &'a VoiceGateway,
    tx: UnboundedSender<Message>,
    seq_ack: Arc<AtomicI64>,
    ssrc: u32,
    udp_addr: Option<SocketAddr>,
    selected_mode: String,
    connected_users: HashSet<UserId>,
    udp_socket: Arc<tokio::net::UdpSocket>,
    dave: Shared<DaveHandler>,
    heartbeat: HeartbeatTracker,
    heartbeat_handle: Option<tokio::task::JoinHandle<()>>,
    conn_token: CancellationToken,
    speaking_tx: Option<UnboundedSender<bool>>,
    session_key: Option<[u8; 32]>,
    speak_task: Option<tokio::task::JoinHandle<()>>,
    persistent_state: Arc<tokio::sync::Mutex<PersistentSessionState>>,
    backoff: &'a mut Backoff,
}

impl<'a> SessionState<'a> {
    /// Creates a new SessionState for a voice gateway connection, initializing networking, session defaults, and backoff tracking.
    ///
    /// This attempts to reuse an existing UDP socket from the provided gateway; if none exists it binds a new non-blocking UDP socket. The returned SessionState is populated with default session values and holds references to the provided gateway, transmit channel, sequence acknowledgment tracker, cancellation token, persistent state, and backoff.
    ///
    /// # Returns
    ///
    /// A new `SessionState` on success, or a `GatewayError` if socket creation or configuration fails.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use tokio::sync::mpsc::unbounded_channel;
    /// # use std::sync::Arc;
    /// # use tokio::sync::Mutex;
    /// # async fn doc_example() -> Result<(), Box<dyn std::error::Error>> {
    /// let (tx, _rx) = unbounded_channel();
    /// let seq_ack = Arc::new(std::sync::atomic::AtomicI64::new(0));
    /// let conn_token = tokio_util::sync::CancellationToken::new();
    /// let persistent_state = Arc::new(Mutex::new(Default::default()));
    /// let mut backoff = crate::gateway::Backoff::default();
    /// // `gateway` must be an available `VoiceGateway` reference in real use:
    /// // let session = SessionState::new(&gateway, tx, seq_ack, conn_token, persistent_state, &mut backoff).await?;
    /// # Ok(()) }
    /// ```
    pub async fn new(
        gateway: &'a VoiceGateway,
        tx: UnboundedSender<Message>,
        seq_ack: Arc<AtomicI64>,
        conn_token: CancellationToken,
        persistent_state: Arc<tokio::sync::Mutex<PersistentSessionState>>,
        backoff: &'a mut Backoff,
    ) -> Result<Self, GatewayError> {
        let mut socket_guard = gateway.udp_socket.lock().await;
        let udp_socket = if let Some(existing) = &*socket_guard {
            existing.clone()
        } else {
            let udp = std::net::UdpSocket::bind("0.0.0.0:0")?;
            udp.set_nonblocking(true)?;
            let socket = Arc::new(tokio::net::UdpSocket::from_std(udp)?);
            *socket_guard = Some(socket.clone());
            socket
        };

        Ok(Self {
            gateway,
            tx,
            seq_ack,
            ssrc: 0,
            udp_addr: None,
            selected_mode: DEFAULT_VOICE_MODE.to_string(),
            connected_users: HashSet::from([gateway.user_id]),
            udp_socket,
            dave: gateway.dave.clone(),
            heartbeat: HeartbeatTracker::new(),
            heartbeat_handle: None,
            conn_token,
            speaking_tx: None,
            session_key: None,
            speak_task: None,
            persistent_state,
            backoff,
        })
    }

    pub fn set_speaking_tx(&mut self, tx: UnboundedSender<bool>) {
        self.speaking_tx = Some(tx);
    }

    pub fn ssrc(&self) -> u32 {
        self.ssrc
    }
    /// Gets a reference to the gateway transmit channel.
    ///
    /// # Examples
    ///
    /// ```
    /// // `session` must be a value with the `tx` accessor in scope.
    /// let tx_ref: &tokio::sync::mpsc::UnboundedSender<crate::gateway::message::Message> = session.tx();
    /// ```
    pub fn tx(&self) -> &UnboundedSender<Message> {
        &self.tx
    }
    /// Returns the current backoff attempt count.
    ///
    /// # Returns
    ///
    /// The current backoff attempt count as a `u32`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// // Obtain a SessionState instance named `session` from your context.
    /// let attempts = session.attempt();
    /// println!("Backoff attempts: {}", attempts);
    /// ```
    pub fn attempt(&self) -> u32 {
        self.backoff.attempt()
    }

    /// Handle an incoming text gateway payload by parsing the JSON and dispatching the matching opcode handler.
    ///
    /// This function parses `text` as a `GatewayPayload`, updates the sequence acknowledgement when present,
    /// and calls the appropriate handler for the payload's opcode. If the payload cannot be parsed, it logs a parse warning and returns `None`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # async fn example(mut session: /* SessionState<'_> */) {
    /// let payload = r#"{"op":2,"d":{}}"#.to_string();
    /// let _ = session.handle_text(payload).await;
    /// # }
    /// ```
    ///
    /// # Returns
    ///
    /// `Some(SessionOutcome)` when a handler indicates a session-level outcome (for example, to re-identify or reconnect), `None` otherwise.
    pub async fn handle_text(&mut self, text: String) -> Option<SessionOutcome> {
        let payload: GatewayPayload = match serde_json::from_str(&text) {
            Ok(p) => p,
            Err(e) => {
                warn!("[{}] JSON Parse error: {e}", self.gateway.guild_id);
                return None;
            }
        };

        if let Some(seq) = payload.seq {
            self.seq_ack.store(seq as i64, Ordering::Relaxed);
        }

        let op = OpCode::from(payload.op);
        trace!(
            "[{}] RX OP: {:?} (op={})",
            self.gateway.guild_id, op, payload.op
        );

        match op {
            OpCode::Hello => self.on_hello(payload.d),
            OpCode::Ready => self.on_ready(payload.d).await,
            OpCode::SessionDescription => self.on_session_description(payload.d).await,
            OpCode::HeartbeatAck => self.on_heartbeat_ack(payload.d),
            OpCode::Resumed => self.on_resumed().await,
            OpCode::ClientConnect => self.on_user_connect(payload.d).await,
            OpCode::ClientDisconnect => self.on_user_disconnect(payload.d).await,
            OpCode::VoiceBackendVersion => {
                info!(
                    "[{}] Voice Backend Version: {:?}",
                    self.gateway.guild_id, payload.d
                );
                None
            }
            OpCode::MediaSinkWants => {
                debug!(
                    "[{}] Media Sink Wants: {:?}",
                    self.gateway.guild_id, payload.d
                );
                None
            }
            OpCode::DavePrepareTransition => self.on_dave_prepare_transition(payload.d).await,
            OpCode::DaveExecuteTransition => self.on_dave_execute_transition(payload.d).await,
            OpCode::DavePrepareEpoch => self.on_dave_prepare_epoch(payload.d).await,
            OpCode::MlsAnnounceCommitTransition => self.on_mls_transition(payload.d).await,
            OpCode::MlsInvalidCommitWelcome => {
                warn!(
                    "[{}] DAVE MLS Invalid Commit Welcome received, resetting session",
                    self.gateway.guild_id
                );
                self.reset_dave(0).await;
                None
            }
            OpCode::NoRoute => {
                warn!(
                    "[{}] No Route received: {:?}",
                    self.gateway.guild_id, payload.d
                );
                None
            }
            OpCode::Speaking
            | OpCode::Video
            | OpCode::Codecs
            | OpCode::UserFlags
            | OpCode::VoicePlatform => None,
            _ => None,
        }
    }

    /// Handle an incoming binary gateway packet and dispatch DAVE-related binary/JSON responses.
    ///
    /// This parses the first two bytes as a big-endian sequence number, the third byte as an opcode,
    /// and the remainder as opcode-specific data. The sequence is recorded in `seq_ack` and the DAVE
    /// handler is locked while processing. Behavior by opcode:
    /// - 25: process external-sender data and send a binary op 28 for each result.
    /// - 27: process proposals; on `Ok(Some(...))` send a binary op 28, on `Err` reset DAVE.
    /// - 29/30: process commit (29) or welcome (30); on success send JSON op 23 with `transition_id`,
    ///   on error reset DAVE using the transition id parsed from the payload when available.
    /// Packets shorter than three bytes are ignored.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # async fn example(sess: &mut SessionState<'_>) {
    /// let seq: u16 = 1;
    /// let op: u8 = 25;
    /// let payload = b"example payload";
    /// let mut bin = Vec::new();
    /// bin.extend_from_slice(&seq.to_be_bytes());
    /// bin.push(op);
    /// bin.extend_from_slice(payload);
    /// sess.handle_binary(bin).await;
    /// # }
    /// ```
    pub async fn handle_binary(&mut self, bin: Vec<u8>) {
        if bin.len() < 3 {
            return;
        }
        let seq = u16::from_be_bytes([bin[0], bin[1]]);
        let op = bin[2];
        let data = &bin[3..];

        self.seq_ack.store(seq as i64, Ordering::Relaxed);
        let mut dave = self.dave.lock().await;

        match op {
            25 => {
                // MlsExternalSender
                if let Ok(res) = dave.process_external_sender(data) {
                    for r in res {
                        self.send_binary(28, &r);
                    }
                }
            }
            27 => {
                // MlsProposals
                match dave.process_proposals(data) {
                    Ok(Some(cw)) => self.send_binary(28, &cw),
                    Err(e) => {
                        warn!("[{}] DAVE proposals failed: {e}", self.gateway.guild_id);
                        self.reset_dave_locked(&mut dave, 0).await;
                    }
                    _ => {}
                }
            }
            29 | 30 => {
                // Commit / Welcome
                let res = if op == 30 {
                    dave.process_welcome(data)
                } else {
                    dave.process_commit(data)
                };
                match res {
                    Ok(tid) if tid != 0 => {
                        self.send_json(23, serde_json::json!({ "transition_id": tid }))
                    }
                    Err(e) => {
                        let tid = if data.len() >= 2 {
                            u16::from_be_bytes([data[0], data[1]])
                        } else {
                            0
                        };
                        warn!(
                            "[{}] DAVE {} failed (tid {tid}): {e}",
                            self.gateway.guild_id,
                            if op == 30 { "welcome" } else { "commit" }
                        );
                        self.reset_dave_locked(&mut dave, tid).await;
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    /// Handle a gateway HELLO payload by starting the heartbeat loop if not already running.
    ///
    /// Expects `d["heartbeat_interval"]` (milliseconds) as an optional unsigned integer; defaults to 30000 ms when absent.
    /// If a heartbeat task is already active, logs a warning and requests re-identification.
    ///
    /// @param d JSON payload from the HELLO event; the only required field is `heartbeat_interval` (optional).
    /// @returns `Some(SessionOutcome::Identify)` when a heartbeat task already exists and re-identification is required, `None` otherwise.
    fn on_hello(&mut self, d: Value) -> Option<SessionOutcome> {
        let interval = d["heartbeat_interval"].as_u64().unwrap_or(30_000);
        if self.heartbeat_handle.is_some() {
            warn!(
                "[{}] Received unexpected mid-session HELLO. Forcing re-identify.",
                self.gateway.guild_id
            );
            return Some(SessionOutcome::Identify);
        }

        trace!(
            "[{}] Heartbeat interval: {interval}ms",
            self.gateway.guild_id
        );

        self.heartbeat_handle = Some(self.heartbeat.spawn(
            self.tx.clone(),
            self.seq_ack.clone(),
            self.conn_token.clone(),
            interval,
        ));
        None
    }

    /// Handle a gateway "Ready" payload: initialize SSRC and UDP address, choose a codec mode,
    /// persist session selection, (optionally) configure DAVE, perform IP discovery, and emit
    /// SelectProtocol/Video/Speaking messages to finalize voice setup.
    ///
    /// The provided JSON payload is expected to contain keys such as `ssrc`, `ip`, `port`,
    /// `modes`, and optionally `dave_protocol_version`. On success this method resets the
    /// backoff state and returns `None`. If IP discovery fails, it returns
    /// `Some(SessionOutcome::Reconnect)`.
    ///
    /// # Parameters
    ///
    /// - `d`: JSON payload received from the gateway containing the Ready information.
    ///
    /// # Returns
    ///
    /// `Some(SessionOutcome::Reconnect)` if IP discovery fails, `None` otherwise.
    ///
    /// # Examples
    ///
    /// ```
    /// use serde_json::json;
    /// use serde_json::Value;
    ///
    /// // Construct a minimal Ready-like payload for inspection.
    /// let d: Value = json!({
    ///     "ssrc": 12345u64,
    ///     "ip": "127.0.0.1",
    ///     "port": 4000u64,
    ///     "modes": ["xsalsa20_poly1305", "aead_aes256_gcm_rtpsize"],
    ///     "dave_protocol_version": 0u64
    /// });
    ///
    /// // `d` can be passed to a SessionState::on_ready invocation in async context.
    /// assert_eq!(d["ssrc"].as_u64().unwrap() as u32, 12345u32);
    /// ```
    async fn on_ready(&mut self, d: Value) -> Option<SessionOutcome> {
        self.ssrc = d["ssrc"].as_u64().unwrap_or(0) as u32;
        let ip = d["ip"].as_str().unwrap_or("");
        let port = d["port"].as_u64().unwrap_or(0) as u16;
        self.udp_addr = Some(format!("{ip}:{port}").parse().ok()?);

        if let Some(modes) = d["modes"].as_array() {
            let pref = ["aead_aes256_gcm_rtpsize", "xsalsa20_poly1305"];
            if let Some(m) = pref
                .iter()
                .find(|&&p| modes.iter().any(|m| m.as_str() == Some(p)))
            {
                self.selected_mode = m.to_string();
            }
        }

        debug!(
            "[{}] Ready: ssrc={}, mode={}",
            self.gateway.guild_id, self.ssrc, self.selected_mode
        );

        {
            let mut state = self.persistent_state.lock().await;
            state.ssrc = self.ssrc;
            state.selected_mode = Some(self.selected_mode.clone());
        }

        if self.gateway.channel_id.0 > 0 {
            let ver = d["dave_protocol_version"]
                .as_u64()
                .unwrap_or(DAVE_INITIAL_VERSION as u64) as u16;
            let mut dave = self.dave.lock().await;
            if ver > 0 {
                dave.set_protocol_version(ver);
                if let Ok(kp) = dave.setup_session(ver) {
                    self.send_binary(26, &kp);
                }
            } else {
                dave.reset();
            }
        }

        match discover_ip(&self.udp_socket, self.udp_addr?, self.ssrc).await {
            Ok((my_ip, my_port)) => {
                self.send_json(OpCode::SelectProtocol as u8, serde_json::json!({
                    "protocol": "udp",
                    "rtc_connection_id": Uuid::new_v4().to_string(),
                    "codecs": [{"name": "opus", "type": "audio", "priority": 1000, "payload_type": 120}],
                    "data": { "address": my_ip, "port": my_port, "mode": self.selected_mode },
                    "address": my_ip,
                    "port": my_port,
                    "mode": self.selected_mode
                }));

                self.send_json(
                    OpCode::Video as u8,
                    serde_json::json!({"audio_ssrc": self.ssrc, "video_ssrc": 0, "rtx_ssrc": 0}),
                );
                self.send_json(
                    OpCode::Speaking as u8,
                    serde_json::json!({"speaking": 0, "delay": 0, "ssrc": self.ssrc}),
                );
            }
            Err(e) => {
                error!("[{}] IP discovery failed: {e}", self.gateway.guild_id);
                return Some(SessionOutcome::Reconnect);
            }
        }

        self.backoff.reset();
        None
    }

    /// Processes a Session Description payload: extracts and stores the 32-byte session key and UDP address,
    /// starts the voice send loop, configures DAVE if the channel is DAVE-enabled, and resets the reconnection backoff.
    ///
    /// The provided JSON `d` is expected to contain a `secret_key` array of 32 numeric entries. On success the
    /// session key and UDP address are persisted and voice sending is started; DAVE is configured when a
    /// nonzero protocol version is present and the gateway's channel indicates DAVE usage.
    ///
    /// # Parameters
    ///
    /// - `d`: JSON session description payload (expects `secret_key` as an array of 32 numbers; optional
    ///   `dave_protocol_version` and `mls_group_id`).
    ///
    /// # Returns
    ///
    /// `None` to continue normal session flow.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use serde_json::json;
    ///
    /// // Build a minimal session description with a 32-byte secret_key.
    /// let secret_key = (0..32).map(|i| json!(i)).collect::<Vec<_>>();
    /// let payload = json!({
    ///     "secret_key": secret_key,
    ///     "dave_protocol_version": 1u64,
    ///     "mls_group_id": 0u64
    /// });
    ///
    /// // `session` is a mutable SessionState; call with `.await` in an async context:
    /// // session.on_session_description(payload).await;
    /// ```
    async fn on_session_description(&mut self, d: Value) -> Option<SessionOutcome> {
        let ka = d["secret_key"].as_array()?;
        let mut key = [0u8; 32];
        for (i, v) in ka.iter().enumerate().take(32) {
            key[i] = v.as_u64()? as u8;
        }

        self.session_key = Some(key);
        let addr = self.udp_addr?;

        {
            let mut state = self.persistent_state.lock().await;
            state.udp_addr = Some(addr);
            state.session_key = Some(key);
            state.ssrc = self.ssrc;
            state.selected_mode = Some(self.selected_mode.clone());
        }

        self.start_voice(addr, key).await;

        if self.gateway.channel_id.0 > 0 {
            let protocol_version = d["dave_protocol_version"]
                .as_u64()
                .unwrap_or(DAVE_INITIAL_VERSION as u64) as u16;
            let mls_group_id = d["mls_group_id"].as_u64().unwrap_or(0);

            let mut dave = self.dave.lock().await;
            if protocol_version > 0 {
                dave.set_protocol_version(protocol_version);
                if let Ok(kp) = dave.setup_session(protocol_version) {
                    self.send_binary(26, &kp);
                }
            } else {
                dave.reset();
            }
            debug!(
                "DAVE setup context: protocol_version={}, mls_group_id={}",
                protocol_version, mls_group_id
            );
        }

        self.backoff.reset();
        None
    }

    /// Restore persisted voice session state after a resume and restart voice streams if the saved state is available.
    ///
    /// If persistent UDP address and session key are present, restores udp_addr, session_key, ssrc, and selected_mode,
    /// starts the voice send loop, and re-announces Video and Speaking state. If the required persistent state is missing,
    /// requests a fresh Identify by returning SessionOutcome::Identify.
    ///
    /// # Returns
    ///
    /// `Some(SessionOutcome::Identify)` if persistent session information is missing and a new identify is required, `None` on successful restore.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // Given a previously constructed `SessionState` named `state`:
    /// async {
    ///     let result = state.on_resumed().await;
    ///     // On success the session is restored and `None` is returned,
    ///     // otherwise `Some(SessionOutcome::Identify)` requests re-identification.
    ///     assert!(matches!(result, None | Some(SessionOutcome::Identify)));
    /// }
    /// ```
    async fn on_resumed(&mut self) -> Option<SessionOutcome> {
        info!("[{}] Resumed", self.gateway.guild_id);
        self.backoff.reset();

        let (addr, key, ssrc, mode) = {
            let state = self.persistent_state.lock().await;
            (
                state.udp_addr,
                state.session_key,
                state.ssrc,
                state.selected_mode.clone(),
            )
        };

        match (addr, key) {
            (Some(addr), Some(key)) => {
                self.udp_addr = Some(addr);
                self.session_key = Some(key);
                self.ssrc = ssrc;
                if let Some(m) = mode {
                    self.selected_mode = m;
                }

                self.start_voice(addr, key).await;
                self.send_json(
                    OpCode::Video as u8,
                    serde_json::json!({"audio_ssrc": self.ssrc, "video_ssrc": 0, "rtx_ssrc": 0}),
                );
                self.send_json(
                    OpCode::Speaking as u8,
                    serde_json::json!({"speaking": 0, "delay": 0, "ssrc": self.ssrc}),
                );
            }
            _ => {
                warn!(
                    "[{}] Resume failed: missing persistent state",
                    self.gateway.guild_id
                );
                return Some(SessionOutcome::Identify);
            }
        }
        None
    }

    /// Process a heartbeat acknowledgement and update session latency state.
    ///
    /// Reads the `t` (nonce) field from the gateway payload and, if it matches a
    /// pending heartbeat, updates the gateway's stored ping (RTT) and resets the
    /// missed-acks counter.
    ///
    /// # Arguments
    ///
    /// * `d` - JSON payload of the heartbeat acknowledgement; expected to contain a numeric `t` field.
    ///
    /// # Returns
    ///
    /// `None` always.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// let payload = serde_json::json!({ "t": 123u64 });
    /// // session is an instance of SessionState; calling this will update session.gateway.ping
    /// // session.on_heartbeat_ack(payload);
    /// ```
    fn on_heartbeat_ack(&self, d: Value) -> Option<SessionOutcome> {
        let nonce = d["t"].as_u64().unwrap_or(0);
        if let Some(rtt) = self.heartbeat.validate_ack(nonce) {
            self.gateway.ping.store(rtt as i64, Ordering::Relaxed);
            self.heartbeat.missed_acks.store(0, Ordering::Relaxed);
        }
        None
    }

    /// Processes a "user connect" gateway payload by adding each numeric user ID in `user_ids`
    /// to the session's connected_users set and notifying the DAVE handler of the new users.
    ///
    /// The JSON payload `d` is expected to contain a `"user_ids"` array of stringified numeric IDs.
    /// Non-numeric or missing entries are ignored.
    ///
    /// # Parameters
    ///
    /// - `d`: JSON object containing the gateway payload data; looks for `d["user_ids"]`.
    ///
    /// # Returns
    ///
    /// `None` (no session outcome).
    ///
    /// # Examples
    ///
    /// ```
    /// // Given a mutable SessionState `state`, call with a payload listing user IDs:
    /// let payload = serde_json::json!({ "user_ids": ["123", "456"] });
    /// // state.on_user_connect(payload).await;
    /// ```
    async fn on_user_connect(&mut self, d: Value) -> Option<SessionOutcome> {
        if let Some(ids) = d["user_ids"].as_array() {
            let mut uids = Vec::new();
            for id in ids {
                if let Some(uid) = id.as_str().and_then(|s| s.parse::<u64>().ok()) {
                    self.connected_users.insert(UserId(uid));
                    uids.push(uid);
                }
            }
            if !uids.is_empty() {
                self.dave.lock().await.add_users(&uids);
            }
        }
        None
    }

    /// Handles a user disconnect payload by removing the user from local tracking and notifying DAVE.
    ///
    /// Expects `d["user_id"]` to be a string containing a decimal user ID. If that field is present and
    /// can be parsed as a `u64`, the corresponding `UserId` is removed from `connected_users` and the
    /// DAVE handler is instructed to remove the user.
    ///
    /// # Parameters
    ///
    /// - `d`: JSON payload from the gateway; should contain `"user_id"` as a string.
    ///
    /// # Returns
    ///
    /// `None` (no session outcome).
    ///
    /// # Examples
    ///
    /// ```
    /// use serde_json::json;
    /// use serde_json::Value;
    ///
    /// // Example payload shape the handler expects:
    /// let payload: Value = json!({ "user_id": "1234567890" });
    /// // The handler will parse the string "1234567890" as a u64 and act accordingly.
    /// ```
    async fn on_user_disconnect(&mut self, d: Value) -> Option<SessionOutcome> {
        if let Some(uid) = d["user_id"].as_str().and_then(|s| s.parse::<u64>().ok()) {
            self.connected_users.remove(&UserId(uid));
            self.dave.lock().await.remove_user(uid);
        }
        None
    }

    /// Handle an incoming DAVE "prepare transition" gateway payload.
    ///
    /// Reads `transition_id` and `protocol_version` from the provided JSON payload,
    /// asks the local DAVE handler to prepare the transition, and if the handler
    /// accepts the preparation sends an acknowledgement JSON payload with the
    /// `transition_id`.
    ///
    /// # Parameters
    ///
    /// - `d`: JSON payload expected to contain numeric `transition_id` and
    ///   `protocol_version` fields.
    ///
    /// # Returns
    ///
    /// `None` when no session-level outcome is produced.
    ///
    /// # Examples
    ///
    /// ```
    /// use serde_json::json;
    /// use serde_json::Value;
    ///
    /// // Construct a payload similar to what the gateway would send.
    /// let payload: Value = json!({ "transition_id": 42, "protocol_version": 1 });
    ///
    /// // The caller would pass `payload` to `on_dave_prepare_transition`.
    /// // This example only demonstrates payload construction and contents.
    /// assert_eq!(payload["transition_id"].as_u64().unwrap(), 42);
    /// assert_eq!(payload["protocol_version"].as_u64().unwrap(), 1);
    /// ```
    async fn on_dave_prepare_transition(&mut self, d: Value) -> Option<SessionOutcome> {
        let tid = d["transition_id"].as_u64().unwrap_or(0) as u16;
        let ver = d["protocol_version"].as_u64().unwrap_or(0) as u16;

        debug!(
            "[{}] DAVE Prepare Transition: id={}, version={}",
            self.gateway.guild_id, tid, ver
        );

        if self.dave.lock().await.prepare_transition(tid, ver) {
            self.send_json(23, serde_json::json!({ "transition_id": tid }));
        }
        None
    }

    /// Executes a DAVE protocol transition specified by the `transition_id` field in the given payload.
    ///
    /// Reads the numeric `transition_id` from the JSON payload `d` (defaults to 0 if missing), instructs
    /// the DAVE handler to execute that transition, and yields no session outcome.
    ///
    /// # Parameters
    ///
    /// - `d`: JSON payload expected to contain a numeric `transition_id` field.
    ///
    /// # Returns
    ///
    /// `None`.
    ///
    /// # Examples
    ///
    /// ```
    /// use serde_json::json;
    ///
    /// // Build a payload containing a transition_id
    /// let payload = json!({ "transition_id": 42 });
    /// // The handler would be called with this payload; here we just demonstrate the payload shape.
    /// assert_eq!(payload["transition_id"].as_u64().unwrap(), 42);
    /// ```
    async fn on_dave_execute_transition(&mut self, d: Value) -> Option<SessionOutcome> {
        let tid = d["transition_id"].as_u64().unwrap_or(0) as u16;
        debug!(
            "[{}] DAVE Execute Transition: id={}",
            self.gateway.guild_id, tid
        );
        self.dave.lock().await.execute_transition(tid);
        None
    }

    /// Handles a DAVE "PrepareEpoch" gateway payload and, if the DAVE handler produces a key payload,
    /// sends it as a binary message (op 26).
    ///
    /// The incoming JSON `d` is expected to contain optional fields `"epoch"` and
    /// `"protocol_version"`; missing values default to 0.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # async fn example(mut session: crate::gateway::session::handler::SessionState<'_>) {
    /// use serde_json::json;
    /// let payload = json!({ "epoch": 42, "protocol_version": 1 });
    /// session.on_dave_prepare_epoch(payload).await;
    /// # }
    /// ```
    ///
    /// # Returns
    ///
    /// `None` — this handler does not produce a session outcome.
    async fn on_dave_prepare_epoch(&mut self, d: Value) -> Option<SessionOutcome> {
        let epoch = d["epoch"].as_u64().unwrap_or(0);
        let ver = d["protocol_version"].as_u64().unwrap_or(0) as u16;
        debug!(
            "[{}] DAVE Prepare Epoch: epoch={}, version={}",
            self.gateway.guild_id, epoch, ver
        );
        if let Some(kp) = self.dave.lock().await.prepare_epoch(epoch, ver) {
            self.send_binary(26, &kp);
        }
        None
    }

    /// Handles an incoming MLS announce commit transition payload from the gateway.
    ///
    /// Reads `transition_id` and optional `protocol_version` from the provided JSON payload.
    /// If `protocol_version` is present, attempts to prepare the DAVE handler for the transition;
    /// when preparation succeeds and `transition_id` is not zero, emits an op 23 JSON message
    /// announcing the transition.
    ///
    /// # Parameters
    ///
    /// - `d`: JSON payload expected to contain `"transition_id"` (numeric) and an optional
    ///   `"protocol_version"` (numeric).
    ///
    /// # Returns
    ///
    /// `None` always.
    ///
    /// # Examples
    ///
    /// ```
    /// // Example usage within an async context:
    /// // session.on_mls_transition(serde_json::json!({"transition_id": 42, "protocol_version": 1})).await;
    /// ```
    async fn on_mls_transition(&mut self, d: Value) -> Option<SessionOutcome> {
        let tid = d["transition_id"].as_u64().unwrap_or(0) as u16;
        debug!(
            "[{}] DAVE MLS Announce Commit Transition: tid={}",
            self.gateway.guild_id, tid
        );
        let ver = d["protocol_version"].as_u64().map(|v| v as u16);
        if let Some(v) = ver {
            let mut dave = self.dave.lock().await;
            if dave.prepare_transition(tid, v) && tid != 0 {
                self.send_json(23, serde_json::json!({ "transition_id": tid }));
            }
        }
        None
    }

    /// Starts or restarts the outgoing voice send loop and announces voice readiness.
    ///
    /// Aborts any currently running speak task, spawns a new speak_loop task configured with
    /// the session's UDP socket, SSRC, selected mode, DAVE handler, and the given socket address
    /// and 32-byte secret key, then sends Video and Speaking gateway messages to announce readiness.
    ///
    /// # Arguments
    ///
    /// * `addr` — Remote UDP socket address to send RTP to.
    /// * `key` — 32-byte secret key used for the voice session.
    ///
    /// # Examples
    ///
    /// ```
    /// # async fn example(mut state: crate::gateway::session::handler::SessionState<'_>) {
    /// use std::net::SocketAddr;
    /// let addr: SocketAddr = "127.0.0.1:5000".parse().unwrap();
    /// state.start_voice(addr, [0u8; 32]).await;
    /// # }
    /// ```
    async fn start_voice(&mut self, addr: SocketAddr, key: [u8; 32]) {
        if let Some(t) = self.speak_task.take() {
            t.abort();
        }

        let config = SpeakConfig {
            mixer: self.gateway.mixer.clone(),
            socket: self.udp_socket.clone(),
            addr,
            ssrc: self.ssrc,
            key,
            mode: self.selected_mode.clone(),
            dave: self.dave.clone(),
            filter_chain: self.gateway.filter_chain.clone(),
            frames_sent: self.gateway.frames_sent.clone(),
            frames_nulled: self.gateway.frames_nulled.clone(),
            cancel_token: self.conn_token.clone(),
            speaking_tx: self.speaking_tx.clone().expect("speaking_tx must be set"),
            persistent_state: self.persistent_state.clone(),
        };

        self.speak_task = Some(tokio::spawn(async move {
            let _ = speak_loop(config).await;
        }));

        self.send_json(
            OpCode::Video as u8,
            serde_json::json!({"audio_ssrc": self.ssrc, "video_ssrc": 0, "rtx_ssrc": 0}),
        );
        self.send_json(
            OpCode::Speaking as u8,
            serde_json::json!({"speaking": 0, "delay": 0, "ssrc": self.ssrc}),
        );
    }

    /// Reset the DAVE handler state for the given transition and emit any necessary gateway messages.
    ///
    /// This acquires the internal DAVE lock and performs a reset sequence for the provided transition
    /// identifier, causing the session to notify peers and, if available, re-send any setup payloads.
    ///
    /// # Examples
    ///
    /// ```
    /// # async fn example(session: &crate::gateway::session::handler::SessionState<'_>) {
    /// session.reset_dave(0).await;
    /// # }
    /// ```
    async fn reset_dave(&self, tid: u16) {
        let mut dave = self.dave.lock().await;
        self.reset_dave_locked(&mut dave, tid).await;
    }

    /// Reset the provided DAVE handler, announce the reset transition, and attempt to reinitialize the DAVE session.
    ///
    /// This calls `dave.reset()`, sends a JSON transition announcement with the given `transition_id`, and if
    /// `dave.setup_session(DAVE_INITIAL_VERSION)` succeeds, sends the resulting key payload as a binary message.
    ///
    /// # Parameters
    ///
    /// - `dave`: mutable reference to the DAVE handler to reset and reinitialize.
    /// - `tid`: transition identifier to include in the announced reset message.
    ///
    /// # Examples
    ///
    /// ```
    /// # async fn example(state: &crate::SessionState<'_>, mut dave: crate::DaveHandler) {
    /// // Reset DAVE and attempt to set up a new session with transition id 0
    /// state.reset_dave_locked(&mut dave, 0).await;
    /// # }
    /// ```
    async fn reset_dave_locked(&self, dave: &mut DaveHandler, tid: u16) {
        dave.reset();
        self.send_json(31, serde_json::json!({ "transition_id": tid }));
        if let Ok(kp) = dave.setup_session(DAVE_INITIAL_VERSION) {
            self.send_binary(26, &kp);
        }
    }

    /// Sends a JSON-encoded GatewayPayload with the given opcode over the session transmit channel.
    ///
    /// The payload is serialized as a GatewayPayload { op, seq: None, d } and sent as a text WebSocket message.
    ///
    /// # Arguments
    ///
    /// * `op` - The gateway opcode to send.
    /// * `d` - The JSON body to include in the payload.
    ///
    /// # Examples
    ///
    /// ```
    /// // Given a `state: SessionState` in scope:
    /// state.send_json(2, serde_json::json!({"action": "select_protocol"}));
    /// ```
    fn send_json(&self, op: u8, d: Value) {
        let _ = self.tx.send(Message::Text(
            serde_json::to_string(&GatewayPayload { op, seq: None, d })
                .unwrap()
                .into(),
        ));
    }

    /// Sends a binary gateway message with `op` as the leading opcode byte.
    ///
    /// The message sent on the session transmit channel is the `op` byte followed by `payload`.
    ///
    /// # Examples
    ///
    /// ```
    /// let op = 0x01u8;
    /// let payload = &[0x02u8, 0x03u8];
    /// let mut encoded = vec![op];
    /// encoded.extend_from_slice(payload);
    /// assert_eq!(encoded, vec![0x01, 0x02, 0x03]);
    /// ```
    fn send_binary(&self, op: u8, payload: &[u8]) {
        let mut b = vec![op];
        b.extend_from_slice(payload);
        let _ = self.tx.send(Message::Binary(b.into()));
    }
}

impl<'a> Drop for SessionState<'a> {
    /// Aborts any background heartbeat and speaking tasks when the session is dropped.
    ///
    /// If a heartbeat task or speak task handle is present, it is taken and aborted to ensure
    /// background tasks do not continue after the session is destroyed.
    fn drop(&mut self) {
        if let Some(h) = self.heartbeat_handle.take() {
            h.abort();
        }
        if let Some(t) = self.speak_task.take() {
            t.abort();
        }
    }
}
