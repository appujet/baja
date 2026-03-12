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
            PCM_FRAME_SAMPLES, SILENCE_FRAME, UDP_KEEPALIVE_GAP_MS, MAX_SILENCE_FRAMES,
        },
        udp_link::UDPVoiceTransport,
    },
};


/// Performs the IP discovery handshake to obtain the remote IP address and port for a given SSRC.
///
/// Sends a discovery UDP packet containing the provided `ssrc` to `addr` and waits (with retries and timeouts)
/// for a discovery response. On success returns the remote IP address encoded in the response and the port
/// parsed from the response bytes.
///
/// # Returns
///
/// A tuple `(ip, port)` where `ip` is the remote IP address as a `String` (trimmed of trailing NULs)
/// and `port` is the remote port as a `u16`.
///
/// # Examples
///
/// ```no_run
/// use std::net::SocketAddr;
/// use tokio::net::UdpSocket;
///
/// #[tokio::main]
/// async fn main() {
///     // Bind a local socket and perform discovery against a gateway address.
///     let socket = UdpSocket::bind("0.0.0.0:0").await.unwrap();
///     let addr: SocketAddr = "198.51.100.1:5000".parse().unwrap();
///     let ssrc: u32 = 123456;
///
///     // discover_ip returns (ip_string, port)
///     let _ = crate::gateway::session::voice::discover_ip(&socket, addr, ssrc).await;
/// }
/// ```
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

        let mut buf = [0u8; DISCOVERY_PACKET_SIZE];
        match tokio::time::timeout(
            Duration::from_secs(IP_DISCOVERY_TIMEOUT_SECS),
            socket.recv(&mut buf),
        )
        .await
        {
            Ok(Ok(n)) if n >= DISCOVERY_PACKET_SIZE => {
                let ip = std::str::from_utf8(&buf[8..72])
                    .map_err(|e| GatewayError::Discovery(e.to_string()))?
                    .trim_matches('\0')
                    .to_string();
                let port = u16::from_be_bytes([buf[72], buf[73]]);
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

/// Starts and runs the voice sending session using the provided configuration.
///
/// This initializes the UDP transport and Opus encoder, constructs a `VoiceSession`,
/// and runs the session until the configuration's cancellation token is triggered or an error occurs.
///
/// # Examples
///
/// ```
/// # use std::sync::Arc;
/// # use tokio::net::UdpSocket;
/// # use std::net::SocketAddr;
/// # use tokio::sync::Mutex;
/// # use my_crate::gateway::session::voice::{SpeakConfig, speak_loop};
/// # use my_crate::gateway::session::voice::GatewayError;
/// # // The following is a minimal, non-functional example showing invocation.
/// # #[tokio::main]
/// # async fn main() -> Result<(), GatewayError> {
/// let config = /* build or obtain a SpeakConfig */ unimplemented!();
/// speak_loop(config).await?;
/// # Ok(())
/// # }
/// ```
///
/// # Returns
///
/// `Ok(())` when the session stops cleanly, or a `GatewayError` if initialization or runtime operation fails.
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
    /// Creates a new VoiceSession initialized with the given configuration and transport.
    ///
    /// The session starts with speaking disabled, no speaking holdoff, the last-transmit
    /// timestamp set to now, and zero active silence frames.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use crate::gateway::session::voice::{SpeakConfig, UDPVoiceTransport, VoiceSession};
    /// let config: SpeakConfig = /* build SpeakConfig */ unimplemented!();
    /// let transport: UDPVoiceTransport = /* build UDPVoiceTransport */ unimplemented!();
    /// let session = VoiceSession::new(config, transport);
    /// ```
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

    /// Runs the voice transmission loop until the session is cancelled.
    ///
    /// The loop ticks at FRAME_DURATION_MS, processes and sends audio frames via `tick`, and periodically persists the current RTP state into `persistent_state.rtp_state`. When the cancellation token is triggered the loop stores the final RTP state and exits.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # async fn example(mut session: crate::gateway::session::VoiceSession, mut encoder: crate::audio::Encoder) {
    /// session.run(&mut encoder).await.unwrap();
    /// # }
    /// ```
    ///
    /// Returns `Ok(())` on normal completion, `Err(GatewayError)` if an error occurs during ticking or state persistence.
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

    /// Performs up to 10 micro-iterations of per-frame audio processing and transmission.
    ///
    /// This method runs the core per-frame logic: it first attempts to produce a frame from the
    /// filter chain's timescale buffer, then falls back to any bypassed Opus frames from the mixer,
    /// and finally mixes PCM input, applies filters, and either encodes/transmits audio or sends a
    /// silence packet depending on activity and holdoff/keepalive timing. Processing stops early
    /// when no further work is available or when a transmission path is taken; otherwise it will
    /// loop at most 10 times before returning.
    ///
    /// # Returns
    ///
    /// `Ok(())` if processing completed without transmission errors; `Err(GatewayError)` if an
    /// underlying send, encryption, or encoding operation failed.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// // Example usage pattern (types elided for brevity):
    /// // async fn example(session: &mut VoiceSession, encoder: &mut Encoder) -> Result<(), GatewayError> {
    /// //     let mut pcm = vec![0i16; 960];
    /// //     let mut opus = vec![0u8; 4000];
    /// //     let mut ts_pcm = vec![0i16; 960];
    /// //     session.tick(encoder, &mut pcm, &mut opus, &mut ts_pcm).await?;
    /// //     Ok(())
    /// // }
    /// ```
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
                    return self.send_silence().await;
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
            } else {
                if self.active_silence > 0 {
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
                    return self.send_silence().await;
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

    /// Update the session's speaking state and notify listeners when it changes.
    ///
    /// When `speaking` differs from the current state, the method sets the new state,
    /// sends the updated value over `speaking_tx`, and if `speaking` is `true` sets
    /// `speaking_holdoff` to `true`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// // Assume `session` is a mutable VoiceSession.
    /// // This marks the session as speaking and notifies listeners.
    /// session.set_speaking(true);
    /// ```
    fn set_speaking(&mut self, speaking: bool) {
        if speaking != self.is_speaking {
            self.is_speaking = speaking;
            let _ = self.config.speaking_tx.send(speaking);
            if speaking {
                self.speaking_holdoff = true;
            }
        }
    }

    /// Encodes a PCM frame to Opus and transmits the resulting bytes, or sends a silence frame if encoding yields no output.
    ///
    /// Attempts to encode `pcm` into the provided `opus` buffer using `encoder`. If encoding produces bytes, those bytes are transmitted;
    /// otherwise a silence frame is transmitted.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # use tokio::runtime::Runtime;
    /// # let rt = Runtime::new().unwrap();
    /// # rt.block_on(async {
    /// // `session` is a mutable VoiceSession; `encoder`, `pcm`, and `opus` are prepared appropriately.
    /// // session.send_pcm(&mut encoder, &pcm_samples, &mut opus_buf).await?;
    /// # Ok(())
    /// # })
    /// # }
    /// ```
    ///
    /// # Returns
    ///
    /// `Ok(())` on success, or a `GatewayError` if transmission fails.
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

    /// Sends the configured RTP silence frame and increments the nulled-frames counter.
    ///
    /// Increments `frames_nulled` and transmits the constant `SILENCE_FRAME` via the session's
    /// transport.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use std::sync::Arc;
    /// # async fn example(mut session: crate::gateway::session::VoiceSession) -> Result<(), crate::gateway::GatewayError> {
    /// session.send_silence().await?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// Returns `Ok(())` if the silence frame was sent (or encryption was skipped), or a `GatewayError` if transmission failed.
    async fn send_silence(&mut self) -> Result<(), GatewayError> {
        self.config.frames_nulled.fetch_add(1, Ordering::Relaxed);
        self.send_raw(&SILENCE_FRAME).await
    }

    /// Encrypts and transmits a single Opus frame.
    ///
    /// Attempts to encrypt `data` using the configured crypto handler; if encryption
    /// succeeds the encrypted payload is transmitted via the transport and the
    /// session transmit timestamp is updated. If encryption fails, nothing is
    /// transmitted and the call still succeeds.
    ///
    /// # Parameters
    ///
    /// - `data`: Opus-encoded audio frame to encrypt and send.
    ///
    /// # Returns
    ///
    /// `Ok(())` if the call completed locally; transport or transmit errors are
    /// returned as `GatewayError`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use tokio::runtime::Runtime;
    /// # async fn _example(mut session: VoiceSession) -> Result<(), GatewayError> {
    /// session.send_raw(&[0u8; 10]).await?;
    /// # Ok(())
    /// # }
    /// ```
    async fn send_raw(&mut self, data: &[u8]) -> Result<(), GatewayError> {
        let mut dave = self.config.dave.lock().await;
        if let Ok(encrypted) = dave.encrypt_opus(data) {
            drop(dave);
            self.transport.transmit_opus(&encrypted).await?;
            self.last_tx_time = Instant::now();
        }
        Ok(())
    }

    /// Resets the session's remaining silence-frame counter to `MAX_SILENCE_FRAMES`.
    ///
    /// This replenishes how many consecutive silent frames the session will treat as active
    /// before switching to explicit silence transmission.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use crate::gateway::session::voice::{VoiceSession, SpeakConfig, UDPVoiceTransport};
    /// # let config = unimplemented!();
    /// # let transport = unimplemented!();
    /// let mut session = VoiceSession::new(config, transport);
    /// session.reset_timers();
    /// ```
    fn reset_timers(&mut self) {
        self.active_silence = MAX_SILENCE_FRAMES;
    }
}
