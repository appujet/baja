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
    pub fn tx(&self) -> &UnboundedSender<Message> {
        &self.tx
    }
    pub fn attempt(&self) -> u32 {
        self.backoff.attempt()
    }
    pub fn has_heartbeat(&self) -> bool {
        self.heartbeat_handle.is_some()
    }

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

    async fn on_ready(&mut self, d: Value) -> Option<SessionOutcome> {
        let ssrc = d["ssrc"].as_u64();
        let ip = d["ip"].as_str();
        let port = d["port"].as_u64();

        match (ssrc, ip, port) {
            (Some(ssrc), Some(ip), Some(port)) if port <= 65535 => {
                self.ssrc = ssrc as u32;
                let addr_str = format!("{ip}:{port}");
                match addr_str.parse::<SocketAddr>() {
                    Ok(addr) => self.udp_addr = Some(addr),
                    Err(_) => {
                        error!(
                            "[{}] Invalid READY address: {addr_str}",
                            self.gateway.guild_id
                        );
                        return Some(SessionOutcome::Reconnect);
                    }
                }
            }
            _ => {
                error!("[{}] Malformed READY payload", self.gateway.guild_id);
                return Some(SessionOutcome::Reconnect);
            }
        }

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

        let target_addr = match self.udp_addr {
            Some(a) => a,
            None => return Some(SessionOutcome::Reconnect),
        };

        match discover_ip(&self.udp_socket, target_addr, self.ssrc).await {
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

    async fn on_session_description(&mut self, d: Value) -> Option<SessionOutcome> {
        let ka = match d["secret_key"].as_array() {
            Some(a) if a.len() == 32 => a,
            _ => {
                error!(
                    "[{}] Invalid or missing secret_key in VOICE_READY",
                    self.gateway.guild_id
                );
                return Some(SessionOutcome::Reconnect);
            }
        };

        let mut key = [0u8; 32];
        for (i, v) in ka.iter().enumerate() {
            if let Some(val) = v.as_u64()
                && val <= 255
            {
                key[i] = val as u8;
                continue;
            }
            error!(
                "[{}] Invalid secret_key byte at index {i}",
                self.gateway.guild_id
            );
            return Some(SessionOutcome::Reconnect);
        }

        self.session_key = Some(key);
        let addr = match self.udp_addr {
            Some(a) => a,
            None => return Some(SessionOutcome::Reconnect),
        };

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

    fn on_heartbeat_ack(&self, d: Value) -> Option<SessionOutcome> {
        let nonce = d["t"].as_u64().unwrap_or(0);
        if let Some(rtt) = self.heartbeat.validate_ack(nonce) {
            self.gateway.ping.store(rtt as i64, Ordering::Relaxed);
            self.heartbeat.missed_acks.store(0, Ordering::Relaxed);
        }
        None
    }

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

    async fn on_user_disconnect(&mut self, d: Value) -> Option<SessionOutcome> {
        if let Some(uid) = d["user_id"].as_str().and_then(|s| s.parse::<u64>().ok()) {
            self.connected_users.remove(&UserId(uid));
            self.dave.lock().await.remove_user(uid);
        }
        None
    }

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

    async fn on_dave_execute_transition(&mut self, d: Value) -> Option<SessionOutcome> {
        let tid = d["transition_id"].as_u64().unwrap_or(0) as u16;
        debug!(
            "[{}] DAVE Execute Transition: id={}",
            self.gateway.guild_id, tid
        );
        self.dave.lock().await.execute_transition(tid);
        None
    }

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

    async fn start_voice(&mut self, addr: SocketAddr, key: [u8; 32]) {
        if let Some(t) = self.speak_task.take() {
            t.abort();
        }

        let speaking_tx = if let Some(tx) = &self.speaking_tx {
            tx.clone()
        } else {
            error!(
                "[{}] speaking_tx is missing, cannot start voice",
                self.gateway.guild_id
            );
            return;
        };

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
            speaking_tx,
            persistent_state: self.persistent_state.clone(),
        };

        let guild_id = self.gateway.guild_id.clone();
        let conn_token = self.conn_token.clone();
        self.speak_task = Some(tokio::spawn(async move {
            if let Err(e) = speak_loop(config).await {
                error!("[{guild_id}] speak_loop failed: {e}");
                conn_token.cancel();
            }
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

    async fn reset_dave(&self, tid: u16) {
        let mut dave = self.dave.lock().await;
        self.reset_dave_locked(&mut dave, tid).await;
    }

    async fn reset_dave_locked(&self, dave: &mut DaveHandler, tid: u16) {
        dave.reset();
        self.send_json(31, serde_json::json!({ "transition_id": tid }));
        if let Ok(kp) = dave.setup_session(DAVE_INITIAL_VERSION) {
            self.send_binary(26, &kp);
        }
    }

    fn send_json(&self, op: u8, d: Value) {
        let _ = self.tx.send(Message::Text(
            serde_json::to_string(&GatewayPayload { op, seq: None, d })
                .unwrap()
                .into(),
        ));
    }

    fn send_binary(&self, op: u8, payload: &[u8]) {
        let mut b = vec![op];
        b.extend_from_slice(payload);
        let _ = self.tx.send(Message::Binary(b.into()));
    }
}

impl<'a> Drop for SessionState<'a> {
    fn drop(&mut self) {
        if let Some(h) = self.heartbeat_handle.take() {
            h.abort();
        }
        if let Some(t) = self.speak_task.take() {
            t.abort();
        }
    }
}
