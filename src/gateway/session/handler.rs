use std::{
    collections::HashSet,
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    },
};

use serde_json::Value;
use tokio::sync::{Mutex, mpsc::UnboundedSender};
use tokio_tungstenite::tungstenite::protocol::Message;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, trace, warn};
use uuid::Uuid;

use super::{
    VoiceGateway,
    backoff::Backoff,
    heartbeat::HeartbeatTracker,
    types::{PersistentSessionState, SessionOutcome, VoiceGatewayMessage, VoiceOp},
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
    speaking_tx: UnboundedSender<bool>,
    session_key: Option<[u8; 32]>,
    speak_task: Option<tokio::task::JoinHandle<()>>,
    persistent_state: Arc<tokio::sync::Mutex<PersistentSessionState>>,
    backoff: &'a mut Backoff,
}

impl<'a> SessionState<'a> {
    pub fn new(
        gateway: &'a VoiceGateway,
        tx: UnboundedSender<Message>,
        seq_ack: Arc<AtomicI64>,
        conn_token: CancellationToken,
        speaking_tx: UnboundedSender<bool>,
        persistent_state: Arc<tokio::sync::Mutex<PersistentSessionState>>,
        backoff: &'a mut Backoff,
    ) -> Result<Self, std::io::Error> {
        let mut users = HashSet::new();
        users.insert(gateway.user_id);

        let udp = std::net::UdpSocket::bind("0.0.0.0:0")?;
        udp.set_nonblocking(true)?;
        let udp_socket = Arc::new(tokio::net::UdpSocket::from_std(udp)?);

        Ok(Self {
            gateway,
            tx,
            seq_ack,
            ssrc: 0,
            udp_addr: None,
            selected_mode: DEFAULT_VOICE_MODE.to_string(),
            connected_users: users,
            udp_socket,
            dave: Arc::new(Mutex::new(DaveHandler::new(
                gateway.user_id,
                gateway.channel_id,
            ))),
            heartbeat: HeartbeatTracker::new(),
            heartbeat_handle: None,
            conn_token,
            speaking_tx,
            session_key: None,
            speak_task: None,
            persistent_state,
            backoff,
        })
    }

    pub fn ssrc(&self) -> u32 {
        self.ssrc
    }
    pub fn tx(&self) -> &UnboundedSender<Message> {
        &self.tx
    }

    pub async fn handle_text(&mut self, text: String) -> Option<SessionOutcome> {
        let msg: VoiceGatewayMessage = match serde_json::from_str(&text) {
            Ok(m) => m,
            Err(e) => {
                warn!("[{}] Parse error: {e}", self.gateway.guild_id);
                return None;
            }
        };

        if let Ok(v) = serde_json::from_str::<Value>(&text)
            && let Some(seq) = v["seq"].as_i64()
        {
            self.seq_ack.store(seq, Ordering::Relaxed);
        }

        let op = VoiceOp::from_raw(msg.op);
        trace!(
            "[{}] WS RX: op={} d={:?}",
            self.gateway.guild_id, msg.op, msg.d
        );

        match op {
            Some(VoiceOp::Hello) => self.handle_hello(msg.d),
            Some(VoiceOp::Ready) => self.handle_ready(msg.d).await,
            Some(VoiceOp::SessionDescription) => self.handle_session_description(msg.d).await,
            Some(VoiceOp::HeartbeatAck) => self.handle_heartbeat_ack(msg.d),
            Some(VoiceOp::Resumed) => self.handle_resumed().await,
            Some(VoiceOp::ClientConnect) => self.handle_user_connect(msg.d),
            Some(VoiceOp::ClientDisconnect) => self.handle_user_disconnect(msg.d),
            Some(
                VoiceOp::Speaking
                | VoiceOp::Video
                | VoiceOp::Codecs
                | VoiceOp::MediaSinkWants
                | VoiceOp::VoiceBackendVersion
                | VoiceOp::VoiceFlags
                | VoiceOp::VoicePlatform,
            ) => None, // Ignore informational events
            Some(VoiceOp::DavePrepareTransition) => self.handle_prepare_transition(msg.d).await,
            Some(VoiceOp::DaveExecuteTransition) => self.handle_execute_transition(msg.d).await,
            Some(VoiceOp::DavePrepareEpoch) => self.handle_prepare_epoch(msg.d).await,
            Some(_) => None, // Ignore other ops
            None => {
                warn!(
                    "[{}] Unhandled op {}: {:?}",
                    self.gateway.guild_id, msg.op, msg.d
                );
                None
            }
        }
    }

    pub async fn handle_binary(&mut self, bin: Vec<u8>) {
        if bin.len() < 3 {
            warn!(
                "[{}] Binary too short: {} bytes",
                self.gateway.guild_id,
                bin.len()
            );
            return;
        }

        let seq = u16::from_be_bytes([bin[0], bin[1]]);
        let op = bin[2];
        let payload = &bin[3..];
        self.seq_ack.store(seq as i64, Ordering::Relaxed);

        let mut dave = self.dave.lock().await;
        match op {
            25 => {
                if let Ok(resps) = dave.process_external_sender(payload, &self.connected_users) {
                    for resp in resps {
                        self.send_binary(28, &resp);
                    }
                }
            }
            27 => {
                if let Err(e) = self.process_dave_proposals(&mut dave, payload).await {
                    warn!("[{}] DAVE proposals failed: {e}", self.gateway.guild_id);
                    self.reset_dave_session(&mut dave, 0).await;
                }
            }
            29 | 30 => {
                let is_welcome = op == 30;
                let res = if is_welcome {
                    dave.process_welcome(payload)
                } else {
                    dave.process_commit(payload)
                };

                match res {
                    Ok(tid) if tid != 0 => {
                        self.send_json(23, serde_json::json!({ "transition_id": tid }))
                    }
                    Err(e) => {
                        let tid = if payload.len() >= 2 {
                            u16::from_be_bytes([payload[0], payload[1]])
                        } else {
                            0
                        };
                        warn!(
                            "[{}] DAVE {} failed (tid {tid}): {e}",
                            self.gateway.guild_id,
                            if is_welcome { "welcome" } else { "commit" }
                        );
                        self.reset_dave_session(&mut dave, tid).await;
                    }
                    _ => {}
                }
            }
            _ => trace!(
                "[{}] Unknown binary op {op} (seq {seq})",
                self.gateway.guild_id
            ),
        }
    }

    async fn process_dave_proposals(
        &self,
        dave: &mut DaveHandler,
        payload: &[u8],
    ) -> crate::common::types::AnyResult<()> {
        if let Some(cw) = dave.process_proposals(payload, &self.connected_users)? {
            self.send_binary(28, &cw);
        }
        Ok(())
    }

    async fn reset_dave_session(&self, dave: &mut DaveHandler, tid: u16) {
        dave.reset();
        self.send_json(31, serde_json::json!({ "transition_id": tid }));
        if let Ok(kp) = dave.setup_session(DAVE_INITIAL_VERSION) {
            self.send_binary(26, &kp);
        }
    }

    fn handle_hello(&mut self, d: Value) -> Option<SessionOutcome> {
        let interval = d["heartbeat_interval"].as_u64().unwrap_or(30_000);

        if let Some(h) = self.heartbeat_handle.take() {
            h.abort();
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

    async fn handle_ready(&mut self, d: Value) -> Option<SessionOutcome> {
        self.ssrc = d["ssrc"].as_u64().unwrap_or(0) as u32;
        let ip = d["ip"].as_str().unwrap_or("");
        let port = d["port"].as_u64().unwrap_or(0) as u16;
        self.udp_addr = Some(format!("{ip}:{port}").parse().ok()?);

        if let Some(modes) = d["modes"].as_array() {
            let preferred = ["aead_aes256_gcm_rtpsize", "xsalsa20_poly1305"];
            if let Some(found) = preferred
                .iter()
                .find(|&&p| modes.iter().any(|m| m.as_str() == Some(p)))
            {
                self.selected_mode = found.to_string();
            }
        }

        let addr = self.udp_addr?;
        debug!(
            "[{}] Ready: ssrc={}, mode={}",
            self.gateway.guild_id, self.ssrc, self.selected_mode
        );

        {
            let mut state = self.persistent_state.lock().await;
            state.ssrc = self.ssrc;
        }

        match discover_ip(&self.udp_socket, addr, self.ssrc).await {
            Ok((my_ip, my_port)) => {
                let rtc_connection_id = Uuid::new_v4().to_string();
                self.send_json(
                    1,
                    serde_json::json!({
                        "protocol": "udp",
                        "rtc_connection_id": rtc_connection_id,
                        "codecs": [
                            {
                                "name": "opus",
                                "type": "audio",
                                "priority": 1000,
                                "payload_type": 120
                            }
                        ],
                        "data": { "address": my_ip, "port": my_port, "mode": self.selected_mode },
                        "address": my_ip,
                        "port": my_port,
                        "mode": self.selected_mode
                    }),
                );

                self.send_json(
                    12,
                    serde_json::json!({
                        "audio_ssrc": self.ssrc,
                        "video_ssrc": 0,
                        "rtx_ssrc": 0,
                    }),
                );
                self.send_json(
                    5,
                    serde_json::json!({
                        "speaking": 0,
                        "delay": 0,
                        "ssrc": self.ssrc,
                    }),
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

    async fn handle_session_description(&mut self, d: Value) -> Option<SessionOutcome> {
        if let Some(m) = d["mode"].as_str() {
            self.selected_mode = m.to_string();
        }

        let secret_key = d["secret_key"].as_array().and_then(|ka| {
            if ka.len() < 32 {
                return None;
            }
            let mut key = [0u8; 32];
            for (i, v) in ka.iter().enumerate().take(32) {
                key[i] = v.as_u64().unwrap_or(0) as u8;
            }
            Some(key)
        });

        let key = match secret_key {
            Some(k) => k,
            None => {
                error!("[{}] Missing secret_key", self.gateway.guild_id);
                return Some(SessionOutcome::Reconnect);
            }
        };

        if let Some(addr) = self.udp_addr {
            self.session_key = Some(key);

            {
                let mut state = self.persistent_state.lock().await;
                state.udp_addr = Some(addr);
                state.session_key = Some(key);
                state.ssrc = self.ssrc;
            }

            self.launch_speak_loop(addr, key).await;
            self.send_json(
                5,
                serde_json::json!({"speaking": 0, "delay": 0, "ssrc": self.ssrc}),
            );
        }

        if self.gateway.channel_id.0 > 0 {
            let mut dave = self.dave.lock().await;
            if let Ok(kp) = dave.setup_session(DAVE_INITIAL_VERSION) {
                self.send_binary(26, &kp);
            }
        }

        self.backoff.reset();
        None
    }

    async fn handle_resumed(&mut self) -> Option<SessionOutcome> {
        info!("[{}] Resumed", self.gateway.guild_id);

        self.backoff.reset();

        let (addr, key, ssrc) = {
            let state = self.persistent_state.lock().await;
            (state.udp_addr, state.session_key, state.ssrc)
        };

        match (addr, key) {
            (Some(addr), Some(key)) => {
                self.udp_addr = Some(addr);
                self.session_key = Some(key);
                self.ssrc = ssrc;

                self.launch_speak_loop(addr, key).await;
                self.send_json(
                    5,
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

    fn handle_heartbeat_ack(&self, d: Value) -> Option<SessionOutcome> {
        let nonce = d["t"].as_u64().unwrap_or(0);
        if let Some(rtt) = self.heartbeat.validate_ack(nonce) {
            self.gateway.ping.store(rtt as i64, Ordering::Relaxed);
            self.heartbeat.missed_acks.store(0, Ordering::Relaxed);
        }
        None
    }

    fn handle_user_connect(&mut self, d: Value) -> Option<SessionOutcome> {
        if let Some(ids) = d["user_ids"].as_array() {
            for id in ids {
                if let Some(uid) = id.as_str().and_then(|s| s.parse::<u64>().ok()) {
                    self.connected_users.insert(UserId(uid));
                }
            }
        }
        None
    }

    fn handle_user_disconnect(&mut self, d: Value) -> Option<SessionOutcome> {
        if let Some(uid) = d["user_id"].as_str().and_then(|s| s.parse::<u64>().ok()) {
            self.connected_users.remove(&UserId(uid));
        }
        None
    }

    async fn handle_prepare_transition(&mut self, d: Value) -> Option<SessionOutcome> {
        let tid = d["transition_id"].as_u64().unwrap_or(0) as u16;
        let ver = d["protocol_version"].as_u64().unwrap_or(0) as u16;
        if self.dave.lock().await.prepare_transition(tid, ver) {
            self.send_json(23, serde_json::json!({ "transition_id": tid }));
        }
        None
    }

    async fn handle_execute_transition(&mut self, d: Value) -> Option<SessionOutcome> {
        let tid = d["transition_id"].as_u64().unwrap_or(0) as u16;
        self.dave.lock().await.execute_transition(tid);
        None
    }

    async fn handle_prepare_epoch(&mut self, d: Value) -> Option<SessionOutcome> {
        let epoch = d["epoch"].as_u64().unwrap_or(0);
        let ver = d["protocol_version"].as_u64().unwrap_or(0) as u16;
        self.dave.lock().await.prepare_epoch(epoch, ver);
        None
    }

    async fn launch_speak_loop(&mut self, addr: SocketAddr, key: [u8; 32]) {
        if let Some(prev) = self.speak_task.take() {
            prev.abort();
        }

        debug!("[{}] Launching speak_loop", self.gateway.guild_id);

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
            speaking_tx: self.speaking_tx.clone(),
        };

        self.speak_task = Some(tokio::spawn(async move {
            if let Err(e) = speak_loop(config).await {
                error!("speak_loop failed: {e}");
            }
        }));
    }

    fn send_json(&self, op: u8, d: Value) {
        let msg = VoiceGatewayMessage { op, d };
        if let Ok(json) = serde_json::to_string(&msg) {
            let _ = self.tx.send(Message::Text(json.into()));
        }
    }

    fn send_binary(&self, op: u8, payload: &[u8]) {
        let mut out = vec![op];
        out.extend_from_slice(payload);
        let _ = self.tx.send(Message::Binary(out.into()));
    }
}

impl<'a> Drop for SessionState<'a> {
    fn drop(&mut self) {
        if let Some(h) = self.heartbeat_handle.take() {
            h.abort();
        }
        if let Some(h) = self.speak_task.take() {
            h.abort();
        }
    }
}
