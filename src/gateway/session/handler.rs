use serde_json::Value;
use std::{
    collections::HashSet,
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicI64, AtomicU64, Ordering},
    },
};
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::protocol::Message;
use tracing::{debug, error, info, warn};

use crate::{
    common::types::{Shared, UserId},
    gateway::{
        DaveHandler,
        session::types::{SessionOutcome, VoiceGatewayMessage},
    },
};

use super::{
    VoiceGateway,
    heartbeat::spawn_heartbeat,
    voice::{discover_ip, speak_loop},
};

pub struct SessionState<'a> {
    gateway: &'a VoiceGateway,
    tx: tokio::sync::mpsc::UnboundedSender<Message>,
    seq_ack: Arc<AtomicI64>,
    ssrc: u32,
    udp_addr: Option<SocketAddr>,
    selected_mode: String,
    connected_users: HashSet<UserId>,
    udp_socket: Arc<tokio::net::UdpSocket>,
    dave: Shared<DaveHandler>,
    heartbeat_handle: Option<tokio::task::JoinHandle<()>>,
    last_heartbeat: Arc<AtomicU64>,
}

impl<'a> SessionState<'a> {
    pub fn new(
        gateway: &'a VoiceGateway,
        tx: tokio::sync::mpsc::UnboundedSender<Message>,
        seq_ack: Arc<AtomicI64>,
    ) -> Self {
        let mut connected_users = HashSet::new();
        connected_users.insert(gateway.user_id);

        let udp_socket = std::net::UdpSocket::bind("0.0.0.0:0").expect("Failed to bind UDP socket");
        udp_socket
            .set_nonblocking(true)
            .expect("Failed to set non-blocking");
        let udp_socket = Arc::new(
            tokio::net::UdpSocket::from_std(udp_socket)
                .expect("Failed to convert to tokio UDP socket"),
        );

        Self {
            gateway,
            tx,
            seq_ack,
            ssrc: 0,
            udp_addr: None,
            selected_mode: "xsalsa20_poly1305".to_string(),
            connected_users,
            udp_socket,
            dave: Arc::new(Mutex::new(DaveHandler::new(
                gateway.user_id,
                gateway.channel_id,
            ))),
            heartbeat_handle: None,
            last_heartbeat: Arc::new(AtomicU64::new(0)),
        }
    }

    pub async fn handle_text(&mut self, text: String) -> Option<SessionOutcome> {
        let msg: VoiceGatewayMessage = match serde_json::from_str(&text) {
            Ok(m) => m,
            Err(e) => {
                warn!(
                    "Failed to parse voice gateway message: {} - Text: {}",
                    e, text
                );
                return None;
            }
        };

        if let Some(seq) = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| v["seq"].as_i64())
        {
            self.seq_ack.store(seq, Ordering::Relaxed);
        }

        match msg.op {
            8 => self.handle_hello(msg.d),
            2 => self.handle_ready(msg.d).await,
            4 => self.handle_session_description(msg.d).await,
            6 => self.handle_heartbeat_ack(),
            9 => self.handle_resumed(),
            11 => self.handle_user_connect(msg.d),
            13 => self.handle_user_disconnect(msg.d),
            21 => self.handle_prepare_transition(msg.d).await,
            22 => self.handle_execute_transition(msg.d).await,
            24 => self.handle_prepare_epoch(msg.d).await,
            _ => {
                debug!("Received voice op {}: {:?}", msg.op, msg.d);
                None
            }
        }
    }

    fn handle_hello(&mut self, d: Value) -> Option<SessionOutcome> {
        let interval = d["heartbeat_interval"].as_u64().unwrap_or(30000);
        if let Some(h) = self.heartbeat_handle.take() {
            h.abort();
        }
        self.heartbeat_handle = Some(spawn_heartbeat(
            self.tx.clone(),
            self.seq_ack.clone(),
            self.last_heartbeat.clone(),
            interval,
        ));
        None
    }

    async fn handle_ready(&mut self, d: Value) -> Option<SessionOutcome> {
        self.ssrc = d["ssrc"].as_u64().unwrap_or(0) as u32;
        let ip = d["ip"].as_str().unwrap_or("");
        let port = d["port"].as_u64().unwrap_or(0) as u16;
        self.udp_addr = Some(format!("{}:{}", ip, port).parse().ok()?);

        if let Some(modes) = d["modes"].as_array() {
            let preferred = ["aead_aes256_gcm_rtpsize", "xsalsa20_poly1305"];
            for p in preferred {
                if modes.iter().any(|m| m.as_str() == Some(p)) {
                    self.selected_mode = p.to_string();
                    break;
                }
            }
        }

        if let Some(addr) = self.udp_addr {
            match discover_ip(&self.udp_socket, addr, self.ssrc).await {
                Ok((my_ip, my_port)) => {
                    let select = VoiceGatewayMessage {
                        op: 1,
                        d: serde_json::json!({
                            "protocol": "udp",
                            "data": { "address": my_ip, "port": my_port, "mode": self.selected_mode }
                        }),
                    };
                    if let Ok(json) = serde_json::to_string(&select) {
                        let _ = self.tx.send(Message::Text(json.into()));
                    }
                }
                Err(_) => return Some(SessionOutcome::Reconnect),
            }
        }
        None
    }

    async fn handle_session_description(&mut self, d: Value) -> Option<SessionOutcome> {
        if let Some(m) = d["mode"].as_str() {
            self.selected_mode = m.to_string();
        }
        if let Some(ka) = d["secret_key"].as_array() {
            let mut key = [0u8; 32];
            for (i, v) in ka.iter().enumerate().take(32) {
                key[i] = v.as_u64().unwrap_or(0) as u8;
            }

            if let Some(addr) = self.udp_addr {
                let mixer = self.gateway.mixer.clone();
                let dave_handle = self.dave.clone();
                let socket = self.udp_socket.clone();
                let mode = self.selected_mode.clone();
                let filter_chain = self.gateway.filter_chain.clone();
                let f_sent = self.gateway.frames_sent.clone();
                let f_nulled = self.gateway.frames_nulled.clone();
                let cancel = self.gateway.cancel_token.clone();
                let ssrc = self.ssrc;

                tokio::spawn(async move {
                    if let Err(e) = speak_loop(
                        mixer,
                        socket,
                        addr,
                        ssrc,
                        key,
                        mode,
                        dave_handle,
                        filter_chain,
                        f_sent,
                        f_nulled,
                        cancel,
                    )
                    .await
                    {
                        error!("Voice playback loop error: {}", e);
                    }
                });

                let speaking = VoiceGatewayMessage {
                    op: 5,
                    d: serde_json::json!({"speaking": 1, "delay": 0, "ssrc": self.ssrc}),
                };
                if let Ok(json) = serde_json::to_string(&speaking) {
                    let _ = self.tx.send(Message::Text(json.into()));
                }
            }
        }

        if self.gateway.channel_id.0 > 0 {
            let mut dave = self.dave.lock().await;
            if let Ok(kp) = dave.setup_session(1) {
                let mut bin = vec![26];
                bin.extend_from_slice(&kp);
                let _ = self.tx.send(Message::Binary(bin.into()));
            }
        }
        None
    }

    fn handle_heartbeat_ack(&self) -> Option<SessionOutcome> {
        let sent_ms = self.last_heartbeat.load(Ordering::Relaxed);
        if sent_ms > 0 {
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64;
            self.gateway
                .ping
                .store((now_ms.saturating_sub(sent_ms)) as i64, Ordering::Relaxed);
        }
        None
    }

    fn handle_resumed(&self) -> Option<SessionOutcome> {
        info!(
            "Voice session resumed successfully for guild {}",
            self.gateway.guild_id
        );
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
        if let Some(id) = d["user_id"].as_str().and_then(|s| s.parse::<u64>().ok()) {
            self.connected_users.remove(&UserId(id));
        }
        None
    }

    async fn handle_prepare_transition(&mut self, d: Value) -> Option<SessionOutcome> {
        let tid = d["transition_id"].as_u64().unwrap_or(0) as u16;
        let version = d["protocol_version"].as_u64().unwrap_or(0) as u16;
        if self.dave.lock().await.prepare_transition(tid, version) {
            let ready = VoiceGatewayMessage {
                op: 23,
                d: serde_json::json!({ "transition_id": tid }),
            };
            if let Ok(json) = serde_json::to_string(&ready) {
                let _ = self.tx.send(Message::Text(json.into()));
            }
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
        let version = d["protocol_version"].as_u64().unwrap_or(0) as u16;
        self.dave.lock().await.prepare_epoch(epoch, version);
        None
    }

    pub async fn handle_binary(&mut self, bin: Vec<u8>) {
        if bin.len() < 3 {
            return;
        }
        let seq = u16::from_be_bytes([bin[0], bin[1]]);
        let op = bin[2];
        let payload = &bin[3..];
        self.seq_ack.store(seq as i64, Ordering::Relaxed);

        let mut dave = self.dave.lock().await;
        match op {
            25 => {
                if let Ok(responses) = dave.process_external_sender(payload, &self.connected_users)
                {
                    for resp in responses {
                        let mut out = vec![28];
                        out.extend_from_slice(&resp);
                        let _ = self.tx.send(Message::Binary(out.into()));
                    }
                }
            }
            27 => match dave.process_proposals(payload, &self.connected_users) {
                Ok(Some(cw)) => {
                    let mut out = vec![28];
                    out.extend_from_slice(&cw);
                    let _ = self.tx.send(Message::Binary(out.into()));
                }
                Ok(None) => {}
                Err(_) => {
                    dave.reset();
                    let invalid = VoiceGatewayMessage {
                        op: 31,
                        d: serde_json::json!({ "transition_id": 0 }),
                    };
                    if let Ok(json) = serde_json::to_string(&invalid) {
                        let _ = self.tx.send(Message::Text(json.into()));
                    }
                    if let Ok(kp) = dave.setup_session(1) {
                        let mut out = vec![26];
                        out.extend_from_slice(&kp);
                        let _ = self.tx.send(Message::Binary(out.into()));
                    }
                }
            },
            30 | 29 => {
                let res = if op == 30 {
                    dave.process_welcome(payload)
                } else {
                    dave.process_commit(payload)
                };
                match res {
                    Ok(tid) => {
                        if tid != 0 {
                            let ready = VoiceGatewayMessage {
                                op: 23,
                                d: serde_json::json!({ "transition_id": tid }),
                            };
                            if let Ok(json) = serde_json::to_string(&ready) {
                                let _ = self.tx.send(Message::Text(json.into()));
                            }
                        }
                    }
                    Err(_) => {
                        let tid = if payload.len() >= 2 {
                            u16::from_be_bytes([payload[0], payload[1]])
                        } else {
                            0
                        };
                        dave.reset();
                        let invalid = VoiceGatewayMessage {
                            op: 31,
                            d: serde_json::json!({ "transition_id": tid }),
                        };
                        if let Ok(json) = serde_json::to_string(&invalid) {
                            let _ = self.tx.send(Message::Text(json.into()));
                        }
                        if let Ok(kp) = dave.setup_session(1) {
                            let mut out = vec![26];
                            out.extend_from_slice(&kp);
                            let _ = self.tx.send(Message::Binary(out.into()));
                        }
                    }
                }
            }
            _ => debug!("Received unknown binary op {} (seq {})", op, seq),
        }
    }
}

impl<'a> Drop for SessionState<'a> {
    fn drop(&mut self) {
        if let Some(h) = self.heartbeat_handle.take() {
            h.abort();
        }
    }
}
