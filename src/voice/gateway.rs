use crate::audio::playback::Mixer;
use crate::voice::DaveHandler;
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::net::{SocketAddr, UdpSocket};
use std::sync::Arc;
use tokio::net::UdpSocket as TokioUdpSocket;
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::protocol::Message;
use tracing::{error, info, warn};

#[derive(Serialize, Deserialize, Debug)]
pub struct VoiceGatewayMessage {
    pub op: u8,
    pub d: Value,
}

pub struct VoiceGateway {
    guild_id: String,
    user_id: u64,
    channel_id: u64,
    session_id: String,
    token: String,
    endpoint: String,
    mixer: Arc<Mutex<Mixer>>,
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
    ) -> Self {
        Self {
            guild_id,
            user_id,
            channel_id,
            session_id,
            token,
            endpoint,
            mixer,
        }
    }

    pub async fn run(self) -> Result<(), Box<dyn std::error::Error>> {
        let url = format!("wss://{}/?v=8", self.endpoint);
        info!("Connecting to voice gateway: {}", url);

        let (ws_stream, _) = tokio_tungstenite::connect_async(url).await?;
        let (mut write, mut read) = ws_stream.split();

        // 1. Identify
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
            .send(Message::Text(serde_json::to_string(&identify)?.into()))
            .await?;

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Message>();

        let write_task = tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                if let Err(e) = write.send(msg).await {
                    error!("WS write error: {}", e);
                    break;
                }
            }
        });

        let mut heartbeat_interval = 30000;
        let mut ssrc = 0;
        let mut udp_addr: Option<SocketAddr> = None;
        let mut secret_key: Option<[u8; 32]> = None;
        let mut selected_mode = "xsalsa20_poly1305".to_string();
        let mut connected_users = HashSet::<u64>::new();
        connected_users.insert(self.user_id);

        let udp_socket = UdpSocket::bind("0.0.0.0:0")?;
        udp_socket.set_nonblocking(true)?;

        let dave = Arc::new(Mutex::new(DaveHandler::new(self.user_id, self.channel_id)));
        let tx_hb = tx.clone();
        let mut heartbeat_handle = None;

        while let Some(msg) = read.next().await {
            match msg? {
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

                    match msg.op {
                        8 => {
                            heartbeat_interval =
                                msg.d["heartbeat_interval"].as_u64().unwrap_or(30000);
                            info!("Voice Hello (Op 8). Interval: {}", heartbeat_interval);

                            let tx_hb_inner = tx_hb.clone();
                            heartbeat_handle = Some(tokio::spawn(async move {
                                let mut interval = tokio::time::interval(
                                    tokio::time::Duration::from_millis(heartbeat_interval),
                                );
                                loop {
                                    interval.tick().await;
                                    let hb = VoiceGatewayMessage {
                                        op: 3,
                                        d: serde_json::json!({ "t": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() }),
                                    };
                                    if let Ok(json) = serde_json::to_string(&hb) {
                                        if tx_hb_inner.send(Message::Text(json.into())).is_err() {
                                            break;
                                        }
                                    }
                                }
                            }));
                        }
                        2 => {
                            ssrc = msg.d["ssrc"].as_u64().unwrap_or(0) as u32;
                            let ip = msg.d["ip"].as_str().unwrap_or("");
                            let port = msg.d["port"].as_u64().unwrap_or(0) as u16;
                            udp_addr = Some(format!("{}:{}", ip, port).parse()?);

                            if let Some(modes) = msg.d["modes"].as_array() {
                                let preferred = ["aead_aes256_gcm_rtpsize", "xsalsa20_poly1305"];
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
                                info!("Discovered IP: {}:{}", my_ip, my_port);

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
                                tx.send(Message::Text(serde_json::to_string(&select)?.into()))?;
                            }
                        }
                        4 => {
                            info!("Session Description received (Op 4): {:?}", msg.d);
                            if let Some(m) = msg.d["mode"].as_str() {
                                selected_mode = m.to_string();
                            }

                            if let Some(ka) = msg.d["secret_key"].as_array() {
                                let mut key = [0u8; 32];
                                for (i, v) in ka.iter().enumerate().take(32) {
                                    key[i] = v.as_u64().unwrap_or(0) as u8;
                                }
                                secret_key = Some(key);

                                if let (Some(addr), Some(k)) = (udp_addr, secret_key) {
                                    let mixer = self.mixer.clone();
                                    let dave_clone = dave.clone();
                                    let socket_clone = udp_socket.try_clone()?;
                                    let mode_clone = selected_mode.clone();

                                    info!(
                                        "Starting speak loop for SSRC {} with mode {}",
                                        ssrc, mode_clone
                                    );
                                    tokio::spawn(async move {
                                        if let Err(e) = speak_loop(
                                            mixer,
                                            socket_clone,
                                            addr,
                                            ssrc,
                                            k,
                                            mode_clone,
                                            dave_clone,
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
                                    tx.send(Message::Text(
                                        serde_json::to_string(&speaking)?.into(),
                                    ))?;
                                }
                            }

                            if self.channel_id > 0 {
                                let mut dave_lock = dave.lock().await;
                                match dave_lock.setup_session(1) {
                                    Ok(kp) => {
                                        info!("Sending DAVE Key Package (Op 26)");
                                        let mut bin = vec![26];
                                        bin.extend_from_slice(&kp);
                                        tx.send(Message::Binary(bin.into()))?;
                                    }
                                    Err(e) => error!("Failed to setup DAVE session: {}", e),
                                }
                            }
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
                            let version = msg.d["protocol_version"].as_u64().unwrap_or(0) as u16;
                            let mut dave_lock = dave.lock().await;
                            if dave_lock.prepare_transition(tid, version) {
                                let ready = VoiceGatewayMessage {
                                    op: 23,
                                    d: serde_json::json!({ "transition_id": tid }),
                                };
                                tx.send(Message::Text(serde_json::to_string(&ready)?.into()))?;
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
                            let version = msg.d["protocol_version"].as_u64().unwrap_or(0) as u16;
                            let mut dave_lock = dave.lock().await;
                            dave_lock.prepare_epoch(epoch, version);
                        }
                        _ => {
                            info!("Received voice op {}: {:?}", msg.op, msg.d);
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

                    let mut dave_lock = dave.lock().await;
                    match op {
                        25 => {
                            // EXTERNAL_SENDER
                            if let Err(e) = dave_lock.process_external_sender(payload) {
                                error!("DAVE process external sender error: {}", e);
                            }
                        }
                        27 => {
                            // PROPOSALS
                            match dave_lock.process_proposals(payload, &connected_users) {
                                Ok(Some(cw)) => {
                                    let mut out = vec![28];
                                    out.extend_from_slice(&cw);
                                    tx.send(Message::Binary(out.into()))?;
                                }
                                Ok(None) => {}
                                Err(e) => error!("DAVE process proposals error: {}", e),
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
                                        tx.send(Message::Text(
                                            serde_json::to_string(&ready)?.into(),
                                        ))?;
                                    }
                                }
                                Err(e) => error!("DAVE process welcome error: {}", e),
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
                                        tx.send(Message::Text(
                                            serde_json::to_string(&ready)?.into(),
                                        ))?;
                                    }
                                }
                                Err(e) => error!("DAVE process commit error: {}", e),
                            }
                        }
                        _ => info!("Received unknown binary op {} (seq {})", op, seq),
                    }
                }
                Message::Close(_) => break,
                _ => {}
            }
        }

        if let Some(h) = heartbeat_handle {
            h.abort();
        }
        write_task.abort();
        Ok(())
    }

    async fn discover_ip(
        &self,
        socket: &UdpSocket,
        addr: SocketAddr,
        ssrc: u32,
    ) -> Result<(String, u16), Box<dyn std::error::Error>> {
        let mut packet = [0u8; 74];
        packet[0..2].copy_from_slice(&1u16.to_be_bytes());
        packet[2..4].copy_from_slice(&70u16.to_be_bytes());
        packet[4..8].copy_from_slice(&ssrc.to_be_bytes());

        socket.send_to(&packet, addr)?;

        let mut buf = [0u8; 74];
        let tokio_socket = TokioUdpSocket::from_std(socket.try_clone()?)?;

        let timeout = tokio::time::Duration::from_secs(2);
        match tokio::time::timeout(timeout, tokio_socket.recv(&mut buf)).await {
            Ok(Ok(n)) => {
                if n < 74 {
                    return Err("IP discovery response too short".into());
                }
                let ip_str = std::str::from_utf8(&buf[8..72])?
                    .trim_matches('\0')
                    .to_string();
                let port = u16::from_le_bytes([buf[72], buf[73]]);
                Ok((ip_str, port))
            }
            Ok(Err(e)) => Err(e.into()),
            Err(_) => Err("IP discovery timeout".into()),
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
) -> Result<(), Box<dyn std::error::Error>> {
    use crate::audio::opus::Encoder;
    use crate::voice::udp::UdpBackend;
    let mut encoder = Encoder::new()?;
    let udp = UdpBackend::new(socket, addr, ssrc, key, &mode)?;
    let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(20));
    // Use Burst to catch up if we fall behind, preventing perceived packet loss gaps
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Burst);

    let mut pcm_buf = vec![0i16; 1920];
    let mut opus_buf = vec![0u8; 4000];

    loop {
        interval.tick().await;
        {
            let mut mixer_lock = mixer.lock().await;
            mixer_lock.mix(&mut pcm_buf).await;
        }

        let size = encoder.encode(&pcm_buf, &mut opus_buf)?;
        if size > 0 {
            let mut dave_lock = dave.lock().await;
            let encrypted_opus = dave_lock.encrypt_opus(&opus_buf[..size])?;
            udp.send_opus_packet(&encrypted_opus)?;
        }
    }
}
