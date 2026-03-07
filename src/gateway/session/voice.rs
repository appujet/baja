use std::{
    net::SocketAddr,
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};

use tokio::sync::mpsc::UnboundedSender;
use tokio_util::sync::CancellationToken;
use tracing::error;

use super::types::map_boxed_err;
use crate::{
    audio::{Mixer, engine::Encoder, filters::FilterChain},
    common::types::{AnyResult, Shared},
    gateway::{
        DaveHandler,
        constants::{
            DISCOVERY_PACKET_SIZE, FRAME_DURATION_MS, IP_DISCOVERY_RETRIES,
            IP_DISCOVERY_RETRY_INTERVAL_MS, IP_DISCOVERY_TIMEOUT_SECS, MAX_OPUS_FRAME_SIZE,
            MAX_SILENCE_FRAMES, PCM_FRAME_SAMPLES, UDP_KEEPALIVE_GAP_MS,
        },
        udp_link::VoiceTransport,
    },
};

const PCM_FRAME_SIZE: usize = PCM_FRAME_SAMPLES * 2;
const SILENCE_BATCH_SIZE: u32 = 5;

pub async fn discover_ip(
    socket: &tokio::net::UdpSocket,
    addr: SocketAddr,
    ssrc: u32,
) -> AnyResult<(String, u16)> {
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
                return Err(map_boxed_err(format!("IP discovery send failed: {e}")));
            }
            continue;
        }

        let mut buf = [0u8; DISCOVERY_PACKET_SIZE];
        match tokio::time::timeout(
            Duration::from_secs(IP_DISCOVERY_TIMEOUT_SECS),
            socket.recv(&mut buf),
        )
        .await
        {
            Ok(Ok(n)) if n >= DISCOVERY_PACKET_SIZE => {
                let ip = std::str::from_utf8(&buf[8..72])
                    .map_err(map_boxed_err)?
                    .trim_matches('\0')
                    .to_string();
                let port = u16::from_be_bytes([buf[72], buf[73]]);
                return Ok((ip, port));
            }
            _ => {
                if attempt == IP_DISCOVERY_RETRIES {
                    return Err(map_boxed_err("IP discovery timeout or invalid response"));
                }
            }
        }
    }

    Err(map_boxed_err("IP discovery exhausted all retries"))
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
}

pub async fn speak_loop(config: SpeakConfig) -> AnyResult<()> {
    let mut encoder = Encoder::new().map_err(map_boxed_err)?;
    let transport = VoiceTransport::new(
        config.socket.clone(),
        config.addr,
        config.ssrc,
        config.key,
        &config.mode,
    )?;

    let mut session = VoiceSession::new(config, transport);
    session.run(&mut encoder).await
}

struct VoiceSession {
    config: SpeakConfig,
    transport: VoiceTransport,
    is_speaking: bool,
    last_tx_time: Instant,
    active_silence: u32,
    idle_frames: u32,
}

impl VoiceSession {
    fn new(config: SpeakConfig, transport: VoiceTransport) -> Self {
        Self {
            config,
            transport,
            is_speaking: false,
            last_tx_time: Instant::now(),
            active_silence: 0,
            idle_frames: 0,
        }
    }

    async fn run(&mut self, encoder: &mut Encoder) -> AnyResult<()> {
        let mut interval = tokio::time::interval(Duration::from_millis(FRAME_DURATION_MS));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        let mut pcm = vec![0i16; PCM_FRAME_SIZE];
        let mut opus = vec![0u8; MAX_OPUS_FRAME_SIZE];
        let mut ts_pcm = vec![0i16; PCM_FRAME_SIZE];

        while !self.config.cancel_token.is_cancelled() {
            interval.tick().await;
            self.tick(encoder, &mut pcm, &mut opus, &mut ts_pcm).await?;
        }

        Ok(())
    }

    async fn tick(
        &mut self,
        encoder: &mut Encoder,
        pcm: &mut [i16],
        opus: &mut [u8],
        ts_pcm: &mut [i16],
    ) -> AnyResult<()> {
        let frame = self.get_next_frame(pcm).await;

        let actively_streaming = frame.has_input || self.active_silence > 0;
        if actively_streaming != self.is_speaking {
            self.is_speaking = actively_streaming;
            let _ = self.config.speaking_tx.send(self.is_speaking);
        }

        if let Some(opus_data) = frame.bypass_data {
            self.reset_timers();
            self.config.frames_sent.fetch_add(1, Ordering::Relaxed);
            return self.encrypt_and_send(&opus_data).await;
        }

        if !frame.needs_encoding {
            return self.maintain_udp().await;
        }

        if frame.has_input {
            self.reset_timers();
            self.config.frames_sent.fetch_add(1, Ordering::Relaxed);
        } else {
            self.idle_frames += 1;
            self.config.frames_nulled.fetch_add(1, Ordering::Relaxed);

            if self.active_silence > 0 {
                self.active_silence -= 1;
                pcm.fill(0);
            } else if self.idle_frames > MAX_SILENCE_FRAMES {
                return self.maintain_udp().await;
            } else {
                pcm.fill(0);
            }
        }

        let pcm_to_encode = self.apply_filters(pcm, ts_pcm).await;

        let size = match encoder.encode(pcm_to_encode, opus) {
            Ok(s) => s,
            Err(e) => {
                error!("Opus encode failed: {e}");
                0
            }
        };

        if size > 0 {
            self.encrypt_and_send(&opus[..size]).await?;
        } else {
            self.maintain_udp().await?;
        }

        Ok(())
    }

    async fn get_next_frame(&self, pcm: &mut [i16]) -> FrameResult {
        let mut mixer = self.config.mixer.lock().await;
        if let Some(data) = mixer.take_opus_frame() {
            FrameResult {
                bypass_data: Some(data),
                has_input: true,
                needs_encoding: false,
            }
        } else {
            let has_input = mixer.mix(pcm);
            FrameResult {
                bypass_data: None,
                has_input,
                needs_encoding: true,
            }
        }
    }

    async fn apply_filters<'a>(&self, pcm: &'a mut [i16], ts: &'a mut [i16]) -> &'a [i16] {
        let mut filter = self.config.filter_chain.lock().await;
        if !filter.is_active() {
            return pcm;
        }

        filter.process(pcm);
        if filter.has_timescale() && filter.fill_frame(ts) {
            ts
        } else {
            pcm
        }
    }

    async fn encrypt_and_send(&mut self, opus_data: &[u8]) -> AnyResult<()> {
        let dave = self.config.dave.clone();
        let mut guard = dave.lock().await;
        if let Ok(encrypted) = guard.encrypt_opus(opus_data) {
            drop(guard);
            self.transport.transmit_opus(&encrypted).await?;
            self.last_tx_time = Instant::now();
        }
        Ok(())
    }

    async fn maintain_udp(&mut self) -> AnyResult<()> {
        let gap = Duration::from_millis(UDP_KEEPALIVE_GAP_MS);
        if self.last_tx_time.elapsed() >= gap {
            self.transport.send_keepalive().await?;
            self.last_tx_time = Instant::now();
        }
        Ok(())
    }

    fn reset_timers(&mut self) {
        self.idle_frames = 0;
        self.active_silence = SILENCE_BATCH_SIZE;
    }
}

struct FrameResult {
    bypass_data: Option<Arc<Vec<u8>>>,
    has_input: bool,
    needs_encoding: bool,
}
