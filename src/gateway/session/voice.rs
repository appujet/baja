use std::{
    net::SocketAddr,
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};

use tokio::sync::mpsc::UnboundedSender;
use tokio_util::sync::CancellationToken;
use tracing::error;

use super::types::GatewayError;
use crate::{
    audio::{Mixer, engine::Encoder, filters::FilterChain},
    common::types::Shared,
    gateway::{
        DaveHandler,
        constants::{
            DISCOVERY_PACKET_SIZE, FRAME_DURATION_MS, IP_DISCOVERY_RETRIES,
            IP_DISCOVERY_RETRY_INTERVAL_MS, IP_DISCOVERY_TIMEOUT_SECS, MAX_OPUS_FRAME_SIZE,
            MAX_SILENCE_FRAMES, PCM_FRAME_SAMPLES, SILENCE_FRAME, UDP_KEEPALIVE_GAP_MS,
        },
        udp_link::UDPVoiceTransport,
    },
};

pub async fn discover_ip(
    socket: &tokio::net::UdpSocket,
    addr: SocketAddr,
    ssrc: u32,
) -> Result<(String, u16), GatewayError> {
    let mut packet = [0u8; DISCOVERY_PACKET_SIZE];
    packet[0..2].copy_from_slice(&1u16.to_be_bytes());
    packet[2..4].copy_from_slice(&70u16.to_be_bytes());
    packet[4..8].copy_from_slice(&ssrc.to_be_bytes());

    for attempt in 1..=IP_DISCOVERY_RETRIES {
        if attempt > 1 {
            tokio::time::sleep(Duration::from_millis(IP_DISCOVERY_RETRY_INTERVAL_MS)).await;
        }

        if let Err(e) = socket.send_to(&packet, addr).await {
            if attempt == IP_DISCOVERY_RETRIES {
                return Err(GatewayError::Discovery(e.to_string()));
            }
            continue;
        }

        let mut client_buf = [0u8; DISCOVERY_PACKET_SIZE];
        match tokio::time::timeout(
            Duration::from_secs(IP_DISCOVERY_TIMEOUT_SECS),
            socket.recv_from(&mut client_buf),
        )
        .await
        {
            Ok(Ok((n, peer))) if n >= DISCOVERY_PACKET_SIZE => {
                if peer != addr {
                    continue;
                }
                let ip = std::str::from_utf8(&client_buf[8..72])
                    .map_err(|e| GatewayError::Discovery(e.to_string()))?
                    .trim_matches('\0')
                    .to_string();
                let port = u16::from_be_bytes([client_buf[72], client_buf[73]]);
                return Ok((ip, port));
            }
            _ => {
                if attempt == IP_DISCOVERY_RETRIES {
                    return Err(GatewayError::Discovery("Timed out".into()));
                }
            }
        }
    }
    Err(GatewayError::Discovery("Exhausted".into()))
}

pub struct SpeakConfig {
    pub mixer: Shared<Mixer>,
    pub socket: Arc<tokio::net::UdpSocket>,
    pub addr: SocketAddr,
    pub ssrc: u32,
    pub key: [u8; 32],
    pub mode: String,
    pub dave: Shared<DaveHandler>,
    pub filter_chain: Shared<FilterChain>,
    pub frames_sent: Arc<std::sync::atomic::AtomicU64>,
    pub frames_nulled: Arc<std::sync::atomic::AtomicU64>,
    pub cancel_token: CancellationToken,
    pub speaking_tx: UnboundedSender<bool>,
    pub persistent_state: Arc<tokio::sync::Mutex<super::types::PersistentSessionState>>,
}

pub async fn speak_loop(config: SpeakConfig) -> Result<(), GatewayError> {
    let rtp_state = { config.persistent_state.lock().await.rtp_state };
    let transport = UDPVoiceTransport::new(
        config.socket.clone(),
        config.addr,
        config.ssrc,
        config.key,
        &config.mode,
        rtp_state,
    )?;
    let mut encoder = Encoder::new().map_err(|e| GatewayError::Encoding(e.to_string()))?;
    let mut session = VoiceSession::new(config, transport);
    session.run(&mut encoder).await
}

struct VoiceSession {
    config: SpeakConfig,
    transport: UDPVoiceTransport,
    is_speaking: bool,
    speaking_holdoff: bool,
    last_tx_time: Instant,
    active_silence: u32,
}

impl VoiceSession {
    fn new(config: SpeakConfig, transport: UDPVoiceTransport) -> Self {
        Self {
            config,
            transport,
            is_speaking: false,
            speaking_holdoff: false,
            last_tx_time: Instant::now(),
            active_silence: 0,
        }
    }

    async fn run(&mut self, encoder: &mut Encoder) -> Result<(), GatewayError> {
        let mut interval = tokio::time::interval(Duration::from_millis(FRAME_DURATION_MS));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        let mut pcm = vec![0i16; PCM_FRAME_SAMPLES * 2];
        let mut opus = vec![0u8; MAX_OPUS_FRAME_SIZE];
        let mut ts_pcm = vec![0i16; PCM_FRAME_SAMPLES * 2];

        while !self.config.cancel_token.is_cancelled() {
            interval.tick().await;
            self.tick(encoder, &mut pcm, &mut opus, &mut ts_pcm).await?;

            if self
                .config
                .frames_sent
                .load(Ordering::Relaxed)
                .is_multiple_of(100)
            {
                self.config.persistent_state.lock().await.rtp_state = Some(self.transport.rtp);
            }
        }

        self.config.persistent_state.lock().await.rtp_state = Some(self.transport.rtp);
        Ok(())
    }

    async fn tick(
        &mut self,
        encoder: &mut Encoder,
        pcm: &mut [i16],
        opus: &mut [u8],
        ts_pcm: &mut [i16],
    ) -> Result<(), GatewayError> {
        let mut loop_count = 0;

        while loop_count < 10 {
            loop_count += 1;

            let ready_from_ts = {
                let mut filters = self.config.filter_chain.lock().await;
                filters.has_timescale() && filters.fill_frame(ts_pcm)
            };

            if ready_from_ts {
                self.set_speaking(true);
                self.config.frames_sent.fetch_add(1, Ordering::Relaxed);
                if self.speaking_holdoff {
                    self.speaking_holdoff = false;
                    self.send_silence().await?;
                }
                return self.send_pcm(encoder, ts_pcm, opus).await;
            }

            let mut mixer = self.config.mixer.lock().await;

            if let Some(data) = mixer.take_opus_frame() {
                drop(mixer);
                self.reset_timers();
                self.set_speaking(true);
                self.config.frames_sent.fetch_add(1, Ordering::Relaxed);
                if self.speaking_holdoff {
                    self.speaking_holdoff = false;
                    self.send_silence().await?;
                }
                return self.send_raw(&data).await;
            }

            let has_input = mixer.mix(pcm);
            drop(mixer);

            if has_input {
                self.reset_timers();
                self.set_speaking(true);
            } else if self.active_silence > 0 {
                self.active_silence -= 1;
                pcm.fill(0);
                self.set_speaking(true);
            } else {
                self.set_speaking(false);
                if self.last_tx_time.elapsed() >= Duration::from_millis(UDP_KEEPALIVE_GAP_MS) {
                    return self.send_silence().await;
                }
                return Ok(());
            }

            let has_ts = {
                let mut filters = self.config.filter_chain.lock().await;
                filters.process(pcm);
                filters.has_timescale()
            };

            if !has_ts {
                if has_input {
                    self.config.frames_sent.fetch_add(1, Ordering::Relaxed);
                } else {
                    self.config.frames_nulled.fetch_add(1, Ordering::Relaxed);
                }

                if self.speaking_holdoff {
                    self.speaking_holdoff = false;
                    self.send_silence().await?;
                }
                return self.send_pcm(encoder, pcm, opus).await;
            }

            let filled_on_silence = {
                let mut filters = self.config.filter_chain.lock().await;
                !has_input && filters.fill_frame(ts_pcm)
            };

            if !has_input && !filled_on_silence {
                break;
            }
        }

        Ok(())
    }

    fn set_speaking(&mut self, speaking: bool) {
        if speaking != self.is_speaking {
            self.is_speaking = speaking;
            let _ = self.config.speaking_tx.send(speaking);
            if speaking {
                self.speaking_holdoff = true;
            }
        }
    }

    async fn send_pcm(
        &mut self,
        encoder: &mut Encoder,
        pcm: &[i16],
        opus: &mut [u8],
    ) -> Result<(), GatewayError> {
        let size = match encoder.encode(pcm, opus) {
            Ok(s) => s,
            Err(e) => {
                error!("Opus encode failed: {e}");
                0
            }
        };

        if size > 0 {
            self.send_raw(&opus[..size]).await?;
        } else {
            self.send_silence().await?;
        }

        Ok(())
    }

    async fn send_silence(&mut self) -> Result<(), GatewayError> {
        self.config.frames_nulled.fetch_add(1, Ordering::Relaxed);
        self.send_raw(&SILENCE_FRAME).await
    }

    async fn send_raw(&mut self, data: &[u8]) -> Result<(), GatewayError> {
        let mut dave = self.config.dave.lock().await;
        let encrypted = dave
            .encrypt_opus(data)
            .map_err(|e| GatewayError::Encryption(e.to_string()))?;
        drop(dave);
        self.transport.transmit_opus(&encrypted).await?;
        self.last_tx_time = Instant::now();
        Ok(())
    }

    fn reset_timers(&mut self) {
        self.active_silence = MAX_SILENCE_FRAMES;
    }
}
