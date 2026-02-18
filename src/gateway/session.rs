use crate::audio::playback::Mixer;
use crate::gateway::DaveHandler;
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use tokio::net::UdpSocket as TokioUdpSocket;
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::protocol::Message;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

#[derive(Serialize, Deserialize, Debug)]
pub struct VoiceGatewayMessage {
    pub op: u8,
    pub d: Value,
}

/// Close codes that allow reconnection (per Discord voice gateway spec).
fn is_reconnectable_close(code: u16) -> bool {
    matches!(code, 1006 | 4015 | 4009)
}

/// Outcome of a single WS session — tells the outer loop what to do next.
enum SessionOutcome {
    /// Reconnectable disconnect — try Op 7 resume.
    Reconnect,
    /// Fatal close or max errors — stop entirely.
    Shutdown,
}

pub struct VoiceGateway {
    guild_id: String,
    user_id: u64,
    channel_id: u64,
    session_id: String,
    token: String,
    endpoint: String,
    mixer: Arc<Mutex<Mixer>>,
    filter_chain: Arc<Mutex<crate::audio::filters::FilterChain>>,
    cancel_token: CancellationToken,
}

const MAX_RECONNECT_ATTEMPTS: u32 = 5;


fn map_boxed_err<E: std::fmt::Display>(
    e: E,
) -> Box<dyn std::error::Error + Send + Sync> {
    Box::new(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
}

impl Drop for VoiceGateway {
    fn drop(&mut self) {
        self.cancel_token.cancel();
    }
}

impl VoiceGateway {
    pub fn new(
        guild_id: String,
        user_id: u64,
        channel_id: u64,
        session_id: String,
        token: String,
        endpoint: String,
        mixer: Arc<Mutex<Mixer>>,
        filter_chain: Arc<Mutex<crate::audio::filters::FilterChain>>,
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
            cancel_token: CancellationToken::new(),
        }
    }

    /// Main entry point — handles reconnection loop around `run_session`.
    pub async fn run(self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut attempt = 0u32;
        let mut is_resume = false;
        // Shared seq_ack for heartbeat + resume (atomic so heartbeat task can read it)
        let seq_ack = Arc::new(AtomicI64::new(-1));

        loop {
            let outcome = self
                .run_session(is_resume, seq_ack.clone())
                .await;

            match outcome {
                Ok(SessionOutcome::Shutdown) => {
                    tracing::debug!("Voice gateway shutting down cleanly for guild {}", self.guild_id);
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
                    let backoff = std::time::Duration::from_millis(
                        1000 * 2u64.pow((attempt - 1).min(3)),
                    );
                    info!(
                        "Voice gateway reconnecting (attempt {}/{}) in {:?} for guild {}",
                        attempt, MAX_RECONNECT_ATTEMPTS, backoff, self.guild_id
                    );
                    tokio::time::sleep(backoff).await;
                    is_resume = true;
                }
                Err(e) => {
                    // Connection-level errors (e.g. DNS failure, TLS handshake) — retry
                    attempt += 1;
                    if attempt > MAX_RECONNECT_ATTEMPTS {
                        error!(
                            "Voice gateway: connection error after {} attempts for guild {}: {}",
                            MAX_RECONNECT_ATTEMPTS, self.guild_id, e
                        );
                        return Err(e);
                    }
                    let backoff = std::time::Duration::from_millis(
                        1000 * 2u64.pow((attempt - 1).min(3)),
                    );
                    warn!(
                        "Voice gateway connection error (attempt {}/{}): {}. Retrying in {:?}",
                        attempt, MAX_RECONNECT_ATTEMPTS, e, backoff
                    );
                    tokio::time::sleep(backoff).await;
                    is_resume = false; // Fresh identify on connection errors
                }
            }
        }
    }

    /// Run a single WS session. Returns the outcome telling the caller
    /// whether to reconnect or shut down.
    async fn run_session(
        &self,
        is_resume: bool,
        seq_ack: Arc<AtomicI64>,
    ) -> Result<SessionOutcome, Box<dyn std::error::Error + Send + Sync>> {
        let url = format!("wss://{}/?v=8", self.endpoint);
        tracing::debug!("Connecting to voice gateway: {}", url);

        let (ws_stream, _) = tokio_tungstenite::connect_async(&url).await.map_err(map_boxed_err)?;
        let (mut write, mut read) = ws_stream.split();

        // Send Identify (Op 0) or Resume (Op 7)
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
            tracing::debug!("Sending voice Resume (Op 7) for guild {}", self.guild_id);
            write
                .send(Message::Text(serde_json::to_string(&resume).map_err(map_boxed_err)?.into()))
                .await.map_err(map_boxed_err)?;
        } else {
            let identify = VoiceGatewayMessage {
                op: 0,
                d: serde_json::json!({
                    "server_id": self.guild_id,
                    "user_id": self.user_id.to_string(),
                    "session_id": self.session_id,
                    "token": self.token,
                    "max_dave_protocol_version": if self.channel_id > 0 { 1 } else { 0 },
                }),
            };
            write
                .send(Message::Text(serde_json::to_string(&identify).map_err(map_boxed_err)?.into()))
                .await.map_err(map_boxed_err)?;
        }

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Message>();

        // Write task — forwards messages from tx to WS. Exits cleanly when tx is dropped.
        let write_task = tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                if let Err(e) = write.send(msg).await {
                    warn!("Voice WS write error (expected on reconnect): {}", e);
                    break;
                }
            }
        });

        let mut ssrc = 0;
        let mut udp_addr: Option<SocketAddr> = None;
        let mut selected_mode = "xsalsa20_poly1305".to_string();
        let mut connected_users = HashSet::<u64>::new();
        connected_users.insert(self.user_id);

        let udp_socket = UdpSocket::bind("0.0.0.0:0").map_err(map_boxed_err)?;
        udp_socket.set_nonblocking(true).map_err(map_boxed_err)?;

        let dave = Arc::new(Mutex::new(DaveHandler::new(self.user_id, self.channel_id)));
        let tx_hb = tx.clone();
        let mut heartbeat_handle = None;

        let outcome = loop {
            let msg = match read.next().await {
                Some(Ok(msg)) => msg,
                Some(Err(e)) => {
                    // IO error on WS read — this is the "Connection reset by peer" case.
                    // Treat as reconnectable.
                    warn!("Voice WS read error: {}. Will attempt reconnect.", e);
                    break SessionOutcome::Reconnect;
                }
                None => {
                    // Stream ended (clean close without close frame)
                    tracing::debug!("Voice WS stream ended for guild {}", self.guild_id);
                    break SessionOutcome::Reconnect;
                }
            };

            match msg {
                Message::Text(text) => {
                    let msg: VoiceGatewayMessage = match serde_json::from_str(&text) {
                        Ok(m) => m,
                        Err(e) => {
                            warn!(
                                "Failed to parse voice gateway message: {} - Text: {}",
                                e, text
                            );
                            continue;
                        }
                    };

                    // Track sequence for seq_ack (some messages include "seq")
                    if let Some(seq) = serde_json::from_str::<Value>(&text)
                        .ok()
                        .and_then(|v| v["seq"].as_i64())
                    {
                        seq_ack.store(seq, Ordering::Relaxed);
                    }

                    match msg.op {
                        8 => {
                            let heartbeat_interval =
                                msg.d["heartbeat_interval"].as_u64().unwrap_or(30000);
                            tracing::debug!("Voice Hello (Op 8). Interval: {}", heartbeat_interval);

                            // Cancel previous heartbeat if any (e.g. on resume)
                            if let Some(h) = heartbeat_handle.take() {
                                let h: tokio::task::JoinHandle<()> = h;
                                h.abort();
                            }

                            let tx_hb_inner = tx_hb.clone();
                            let seq_ack_hb = seq_ack.clone();
                            heartbeat_handle = Some(tokio::spawn(async move {
                                let mut interval = tokio::time::interval(
                                    tokio::time::Duration::from_millis(heartbeat_interval),
                                );
                                loop {
                                    interval.tick().await;
                                    let current_seq = seq_ack_hb.load(Ordering::Relaxed);
                                    let hb = VoiceGatewayMessage {
                                        op: 3,
                                        d: serde_json::json!({
                                            "t": std::time::SystemTime::now()
                                                .duration_since(std::time::UNIX_EPOCH)
                                                .unwrap()
                                                .as_millis() as u64,
                                            "seq_ack": current_seq
                                        }),
                                    };
                                    if let Ok(json) = serde_json::to_string(&hb) {
                                        if tx_hb_inner
                                            .send(Message::Text(json.into()))
                                            .is_err()
                                        {
                                            break; // Channel closed — session ending
                                        }
                                    }
                                }
                            }));
                        }
                        2 => {
                            ssrc = msg.d["ssrc"].as_u64().unwrap_or(0) as u32;
                            let ip = msg.d["ip"].as_str().unwrap_or("");
                            let port = msg.d["port"].as_u64().unwrap_or(0) as u16;
                            udp_addr = Some(format!("{}:{}", ip, port).parse().map_err(map_boxed_err)?);

                            if let Some(modes) = msg.d["modes"].as_array() {
                                let preferred =
                                    ["aead_aes256_gcm_rtpsize", "xsalsa20_poly1305"];
                                for p in preferred {
                                    if modes.iter().any(|m| m.as_str() == Some(p)) {
                                        selected_mode = p.to_string();
                                        break;
                                    }
                                }
                            }
                            info!(
                                "Voice Ready (Op 2). SSRC: {}, UDP: {:?}, Mode: {}",
                                ssrc, udp_addr, selected_mode
                            );

                            if let Some(addr) = udp_addr {
                                let (my_ip, my_port) =
                                    self.discover_ip(&udp_socket, addr, ssrc).await?;
                                tracing::debug!("Discovered IP: {}:{}", my_ip, my_port);

                                let select = VoiceGatewayMessage {
                                    op: 1,
                                    d: serde_json::json!({
                                        "protocol": "udp",
                                        "data": {
                                            "address": my_ip,
                                            "port": my_port,
                                            "mode": selected_mode
                                        }
                                    }),
                                };
                                if tx
                                    .send(Message::Text(
                                        serde_json::to_string(&select).map_err(map_boxed_err)?.into(),
                                    ))
                                    .is_err()
                                {
                                    break SessionOutcome::Reconnect;
                                }
                            }
                        }
                        4 => {
                            tracing::debug!("Session Description received (Op 4): {:?}", msg.d);
                            if let Some(m) = msg.d["mode"].as_str() {
                                selected_mode = m.to_string();
                            }

                            if let Some(ka) = msg.d["secret_key"].as_array() {
                                let mut key = [0u8; 32];
                                for (i, v) in ka.iter().enumerate().take(32) {
                                    key[i] = v.as_u64().unwrap_or(0) as u8;
                                }

                                if let Some(addr) = udp_addr {
                                    let mixer = self.mixer.clone();
                                    let dave_clone = dave.clone();
                                    let socket_clone = udp_socket.try_clone().map_err(map_boxed_err)?;
                                    let mode_clone = selected_mode.clone();
                                    let cancel_clone = self.cancel_token.clone();

                                    info!(
                                        "Starting speak loop for SSRC {} with mode {}",
                                        ssrc, mode_clone
                                    );
                                    let filter_chain_clone = self.filter_chain.clone();
                                    tokio::spawn(async move {
                                        if let Err(e) = speak_loop(
                                            mixer,
                                            socket_clone,
                                            addr,
                                            ssrc,
                                            key,
                                            mode_clone,
                                            dave_clone,
                                            filter_chain_clone,
                                            cancel_clone,
                                        )
                                        .await
                                        {
                                            error!("Speak loop error: {}", e);
                                        }
                                    });

                                    let speaking = VoiceGatewayMessage {
                                        op: 5,
                                        d: serde_json::json!({"speaking": 1, "delay": 0, "ssrc": ssrc}),
                                    };
                                    if tx
                                        .send(Message::Text(
                                            serde_json::to_string(&speaking).map_err(map_boxed_err)?.into(),
                                        ))
                                        .is_err()
                                    {
                                        break SessionOutcome::Reconnect;
                                    }
                                }
                            }

                            if self.channel_id > 0 {
                                let mut dave_lock = dave.lock().await;
                                match dave_lock.setup_session(1) {
                                    Ok(kp) => {
                                        tracing::debug!("Sending DAVE Key Package (Op 26)");
                                        let mut bin = vec![26];
                                        bin.extend_from_slice(&kp);
                                        let _ = tx.send(Message::Binary(bin.into()));
                                    }
                                    Err(e) => error!("Failed to setup DAVE session: {}", e),
                                }
                            }
                        }
                        6 => {
                            // Heartbeat ACK — can measure ping here if needed
                        }
                        9 => {
                            // Op 9: Resumed — reset reconnect attempts
                            info!("Voice session resumed successfully for guild {}", self.guild_id);
                        }
                        11 => {
                            // USER_CONNECT
                            if let Some(ids) = msg.d["user_ids"].as_array() {
                                for id in ids {
                                    if let Some(uid) =
                                        id.as_str().and_then(|s| s.parse::<u64>().ok())
                                    {
                                        connected_users.insert(uid);
                                    }
                                }
                            }
                        }
                        13 => {
                            // USER_DISCONNECT
                            if let Some(id) = msg.d["user_id"]
                                .as_str()
                                .and_then(|s| s.parse::<u64>().ok())
                            {
                                connected_users.remove(&id);
                            }
                        }
                        21 => {
                            // PREPARE_TRANSITION
                            let tid = msg.d["transition_id"].as_u64().unwrap_or(0) as u16;
                            let version =
                                msg.d["protocol_version"].as_u64().unwrap_or(0) as u16;
                            let mut dave_lock = dave.lock().await;
                            if dave_lock.prepare_transition(tid, version) {
                                let ready = VoiceGatewayMessage {
                                    op: 23,
                                    d: serde_json::json!({ "transition_id": tid }),
                                };
                                let _ = tx.send(Message::Text(
                                    serde_json::to_string(&ready).map_err(map_boxed_err)?.into(),
                                ));
                            }
                        }
                        22 => {
                            // EXECUTE_TRANSITION
                            let tid = msg.d["transition_id"].as_u64().unwrap_or(0) as u16;
                            let mut dave_lock = dave.lock().await;
                            dave_lock.execute_transition(tid);
                        }
                        24 => {
                            // PREPARE_EPOCH
                            let epoch = msg.d["epoch"].as_u64().unwrap_or(0);
                            let version =
                                msg.d["protocol_version"].as_u64().unwrap_or(0) as u16;
                            let mut dave_lock = dave.lock().await;
                            dave_lock.prepare_epoch(epoch, version);
                        }
                        _ => {
                            tracing::debug!("Received voice op {}: {:?}", msg.op, msg.d);
                        }
                    }
                }
                Message::Binary(bin) => {
                    if bin.len() < 3 {
                        continue;
                    }
                    let seq = u16::from_be_bytes([bin[0], bin[1]]);
                    let op = bin[2];
                    let payload = &bin[3..];

                    // Track binary sequence for seq_ack
                    seq_ack.store(seq as i64, Ordering::Relaxed);

                    let mut dave_lock = dave.lock().await;
                    match op {
                        25 => {
                            // EXTERNAL_SENDER
                            match dave_lock.process_external_sender(payload, &connected_users) {
                                Ok(responses) => {
                                    for resp in responses {
                                        let mut out = vec![28];
                                        out.extend_from_slice(&resp);
                                        let _ = tx.send(Message::Binary(out.into()));
                                    }
                                }
                                Err(e) => {
                                    error!("DAVE process external sender error: {}", e);
                                }
                            }
                        }
                        27 => {
                            // PROPOSALS
                            match dave_lock.process_proposals(payload, &connected_users)
                            {
                                Ok(Some(cw)) => {
                                    let mut out = vec![28];
                                    out.extend_from_slice(&cw);
                                    let _ = tx.send(Message::Binary(out.into()));
                                }
                                Ok(None) => {}
                                Err(e) => {
                                    let err_str = e.to_string();
                                    warn!("DAVE process proposals error: {}. Recovering...", err_str);

                                    // Recovery: Reset session -> Op 31 (Invalid) -> Op 26 (Key Package)
                                    dave_lock.reset();
   
                                    // Send Op 31 INVALID_COMMIT_WELCOME
                                    let invalid = VoiceGatewayMessage {
                                        op: 31,
                                        d: serde_json::json!({ "transition_id": 0 }), 
                                    };
                                    let _ = tx.send(Message::Text(serde_json::to_string(&invalid).unwrap().into()));

                                    // Re-advertise Key Package (Op 26)
                                    if let Ok(kp) = dave_lock.setup_session(1) {
                                        let mut bin = vec![26]; // Op 26
                                        bin.extend_from_slice(&kp);
                                        let _ = tx.send(Message::Binary(bin.into()));
                                    }
                                }
                            }
                        }
                        30 => {
                            // WELCOME
                            match dave_lock.process_welcome(payload) {
                                Ok(tid) => {
                                    if tid != 0 {
                                        let ready = VoiceGatewayMessage {
                                            op: 23,
                                            d: serde_json::json!({ "transition_id": tid }),
                                        };
                                        let _ = tx.send(Message::Text(
                                            serde_json::to_string(&ready).map_err(map_boxed_err)?.into(),
                                        ));
                                    }
                                }
                                Err(e) => {
                                    warn!("DAVE process welcome error: {}. Recovering...", e);
                                    
                                    let tid = if payload.len() >= 2 {
                                        u16::from_be_bytes([payload[0], payload[1]])
                                    } else { 0 };

                                    dave_lock.reset();

                                    let invalid = VoiceGatewayMessage {
                                        op: 31,
                                        d: serde_json::json!({ "transition_id": tid }), 
                                    };
                                    let _ = tx.send(Message::Text(serde_json::to_string(&invalid).unwrap().into()));

                                    if let Ok(kp) = dave_lock.setup_session(1) {
                                        let mut bin = vec![26];
                                        bin.extend_from_slice(&kp);
                                        let _ = tx.send(Message::Binary(bin.into()));
                                    }
                                }
                            }
                        }
                        29 => {
                            // ANNOUNCE_COMMIT_TRANSITION
                            match dave_lock.process_commit(payload) {
                                Ok(tid) => {
                                    if tid != 0 {
                                        let ready = VoiceGatewayMessage {
                                            op: 23,
                                            d: serde_json::json!({ "transition_id": tid }),
                                        };
                                        let _ = tx.send(Message::Text(
                                            serde_json::to_string(&ready).map_err(map_boxed_err)?.into(),
                                        ));
                                    }
                                }
                                Err(e) => {
                                    warn!("DAVE process commit error: {}. Recovering...", e);
                                    
                                    let tid = if payload.len() >= 2 {
                                        u16::from_be_bytes([payload[0], payload[1]])
                                    } else { 0 };

                                    dave_lock.reset();

                                    let invalid = VoiceGatewayMessage {
                                        op: 31,
                                        d: serde_json::json!({ "transition_id": tid }), 
                                    };
                                    let _ = tx.send(Message::Text(serde_json::to_string(&invalid).unwrap().into()));

                                    if let Ok(kp) = dave_lock.setup_session(1) {
                                        let mut bin = vec![26];
                                        bin.extend_from_slice(&kp);
                                        let _ = tx.send(Message::Binary(bin.into()));
                                    }
                                }
                            }
                        }
                        _ => tracing::debug!(
                            "Received unknown binary op {} (seq {})",
                            op, seq
                        ),
                    }
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

                    if is_reconnectable_close(code) {
                        break SessionOutcome::Reconnect;
                    } else {
                        break SessionOutcome::Shutdown;
                    }
                }
                _ => {}
            }
        };

        // Clean up: abort heartbeat, drop tx to signal write task to exit, cancel speak loops
        if let Some(h) = heartbeat_handle {
            h.abort();
        }
        self.cancel_token.cancel();
        drop(tx);
        drop(tx_hb);
        // Give the write task a moment to exit cleanly
        let _ = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            write_task,
        )
        .await;

        Ok(outcome)
    }

    async fn discover_ip(
        &self,
        socket: &UdpSocket,
        addr: SocketAddr,
        ssrc: u32,
    ) -> Result<(String, u16), Box<dyn std::error::Error + Send + Sync>> {
        let mut packet = [0u8; 74];
        packet[0..2].copy_from_slice(&1u16.to_be_bytes());
        packet[2..4].copy_from_slice(&70u16.to_be_bytes());
        packet[4..8].copy_from_slice(&ssrc.to_be_bytes());

        socket.send_to(&packet, addr).map_err(map_boxed_err)?;

        let mut buf = [0u8; 74];
        let tokio_socket = TokioUdpSocket::from_std(socket.try_clone().map_err(map_boxed_err)?).map_err(map_boxed_err)?;

        let timeout = tokio::time::Duration::from_secs(2);
        match tokio::time::timeout(timeout, tokio_socket.recv(&mut buf)).await {
            Ok(Ok(n)) => {
                if n < 74 {
                    return Err(map_boxed_err(std::io::Error::new(std::io::ErrorKind::Other, "IP discovery response too short")));
                }
                let ip_str = std::str::from_utf8(&buf[8..72]).map_err(map_boxed_err)?
                    .trim_matches('\0')
                    .to_string();
                let port = u16::from_le_bytes([buf[72], buf[73]]);
                Ok((ip_str, port))
            }
            Ok(Err(e)) => Err(map_boxed_err(e)),
            Err(_) => Err(map_boxed_err(std::io::Error::new(std::io::ErrorKind::TimedOut, "IP discovery timeout"))),
        }
    }
}

async fn speak_loop(
    mixer: Arc<Mutex<Mixer>>,
    socket: UdpSocket,
    addr: SocketAddr,
    ssrc: u32,
    key: [u8; 32],
    mode: String,
    dave: Arc<Mutex<DaveHandler>>,
    filter_chain: Arc<Mutex<crate::audio::filters::FilterChain>>,
    cancel_token: CancellationToken,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use crate::audio::pipeline::encoder::Encoder;
    use crate::gateway::UdpBackend;
    let mut encoder = Encoder::new().map_err(map_boxed_err)?;
    let udp = UdpBackend::new(socket, addr, ssrc, key, &mode).map_err(map_boxed_err)?;
    let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(20));
    // Use Burst to catch up if we fall behind, preventing perceived packet loss gaps
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Burst);

    let mut pcm_buf = vec![0i16; 1920];
    let mut opus_buf = vec![0u8; 4000];
    let mut silence_frames = 0;
    // Timescale output buffer for feeding fixed-size frames to the encoder
    let mut ts_frame_buf = vec![0i16; 1920];

    loop {
        tokio::select! {
            _ = cancel_token.cancelled() => {
                break;
            }
            _ = interval.tick() => {
                let has_audio;
                {
                    let mut mixer_lock = mixer.lock().await;
                    has_audio = mixer_lock.mix(&mut pcm_buf).await;
                }

                if has_audio {
                    silence_frames = 0;
                } else {
                    silence_frames += 1;
                    // Send 5 frames of silence to distinguish "end of speech" from "packet loss"
                    if silence_frames > 5 {
                        continue;
                    }
                }

                // Apply audio filters
                let mut fc = filter_chain.lock().await;
                if fc.is_active() {
                    fc.process(&mut pcm_buf);

                    if fc.has_timescale() {
                        // Timescale changes buffer length — drain fixed frames
                        if fc.fill_frame(&mut ts_frame_buf) {
                            let size = encoder.encode(&ts_frame_buf, &mut opus_buf).map_err(map_boxed_err)?;
                            drop(fc);
                            if size > 0 {
                                let mut dave_lock = dave.lock().await;
                                match dave_lock.encrypt_opus(&opus_buf[..size]) {
                                    Ok(encrypted_opus) => {
                                        if let Err(e) = udp.send_opus_packet(&encrypted_opus) {
                                            tracing::warn!("Failed to send UDP packet: {}", e);
                                        }
                                    }
                                    Err(e) => {
                                        tracing::error!("DAVE encryption failed: {}", e);
                                    }
                                }
                            }
                        } else {
                            // Not enough data yet from timescale — skip frame
                            drop(fc);
                        }
                        continue;
                    }
                }
                drop(fc);

                let size = encoder.encode(&pcm_buf, &mut opus_buf).map_err(map_boxed_err)?;
                if size > 0 {
                    let mut dave_lock = dave.lock().await;
                    match dave_lock.encrypt_opus(&opus_buf[..size]) {
                        Ok(encrypted_opus) => {
                            if let Err(e) = udp.send_opus_packet(&encrypted_opus) {
                                tracing::warn!("Failed to send UDP packet: {}", e);
                            }
                        }
                        Err(e) => {
                            tracing::error!("DAVE encryption failed: {}", e);
                        }
                    }
                }
            }
        }
    }
    Ok(())
}
