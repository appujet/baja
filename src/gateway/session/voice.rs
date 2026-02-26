use super::types::map_boxed_err;
use crate::{
    audio::{Mixer, engine::Encoder},
    common::types::{AnyResult, Shared},
    gateway::{
        DaveHandler, UdpBackend,
        constants::{
            DISCOVERY_PACKET_SIZE, FRAME_DURATION_MS, IP_DISCOVERY_TIMEOUT_SECS,
            MAX_OPUS_FRAME_SIZE, MAX_SILENCE_FRAMES, PCM_FRAME_SAMPLES,
        },
    },
};
use std::{
    net::SocketAddr,
    sync::{Arc, atomic::Ordering},
};
use tokio_util::sync::CancellationToken;
use tracing::error;

const PCM_FRAME_SIZE: usize = PCM_FRAME_SAMPLES * 2; // Stereo

pub async fn discover_ip(
    socket: &tokio::net::UdpSocket,
    addr: SocketAddr,
    ssrc: u32,
) -> AnyResult<(String, u16)> {
    let mut packet = [0u8; DISCOVERY_PACKET_SIZE];
    packet[0..2].copy_from_slice(&1u16.to_be_bytes());
    packet[2..4].copy_from_slice(&70u16.to_be_bytes());
    packet[4..8].copy_from_slice(&ssrc.to_be_bytes());

    socket.send_to(&packet, addr).await.map_err(map_boxed_err)?;

    let mut buf = [0u8; DISCOVERY_PACKET_SIZE];
    match tokio::time::timeout(
        tokio::time::Duration::from_secs(IP_DISCOVERY_TIMEOUT_SECS),
        socket.recv(&mut buf),
    )
    .await
    {
        Ok(Ok(n)) if n >= DISCOVERY_PACKET_SIZE => {
            let ip_str = std::str::from_utf8(&buf[8..72])
                .map_err(map_boxed_err)?
                .trim_matches('\0')
                .to_string();
            let port = u16::from_le_bytes([buf[72], buf[73]]);
            Ok((ip_str, port))
        }
        Ok(Ok(_)) => Err(map_boxed_err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "Malformed IP discovery response",
        ))),
        Ok(Err(e)) => Err(map_boxed_err(e)),
        Err(_) => Err(map_boxed_err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "IP discovery timed out",
        ))),
    }
}

pub async fn speak_loop(
    mixer: Shared<Mixer>,
    socket: Arc<tokio::net::UdpSocket>,
    addr: SocketAddr,
    ssrc: u32,
    key: [u8; 32],
    mode: String,
    dave: Shared<DaveHandler>,
    filter_chain: Shared<crate::audio::filters::FilterChain>,
    frames_sent: Arc<std::sync::atomic::AtomicU64>,
    frames_nulled: Arc<std::sync::atomic::AtomicU64>,
    cancel_token: CancellationToken,
) -> AnyResult<()> {
    let mut encoder = Encoder::new().map_err(map_boxed_err)?;
    let mut udp = UdpBackend::new(socket, addr, ssrc, key, &mode).map_err(map_boxed_err)?;
    let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(FRAME_DURATION_MS));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let mut pcm_buf = vec![0i16; PCM_FRAME_SIZE];
    let mut opus_buf = vec![0u8; MAX_OPUS_FRAME_SIZE];
    let mut silence_frames = 0;
    let mut ts_frame_buf = vec![0i16; PCM_FRAME_SIZE];

    loop {
        tokio::select! {
            _ = cancel_token.cancelled() => break,
            _ = interval.tick() => {
                // 1. Try taking an Opus frame (passthrough)
                let opus_frame = {
                    let mut m = mixer.lock().await;
                    m.take_opus_frame()
                };

                if let Some(frame) = opus_frame {
                    silence_frames = 0;
                    frames_sent.fetch_add(1, Ordering::Relaxed);
                    let mut d = dave.lock().await;
                    if let Ok(packet) = d.encrypt_opus(&frame) {
                        let _ = udp.send_opus_packet(&packet).await;
                    }
                    continue;
                }

                // 2. Mix PCM
                let has_audio = {
                    let mut m = mixer.lock().await;
                    m.mix(&mut pcm_buf)
                };

                if !has_audio {
                    silence_frames += 1;
                    frames_nulled.fetch_add(1, Ordering::Relaxed);
                    if silence_frames > MAX_SILENCE_FRAMES { continue; }
                    pcm_buf.fill(0);
                } else {
                    silence_frames = 0;
                    frames_sent.fetch_add(1, Ordering::Relaxed);
                }

                // 3. Process Effects & Filter Chain
                let mut use_ts = false;
                let mut encode_ready = true;
                {
                    let mut fc = filter_chain.lock().await;
                    if fc.is_active() {
                        fc.process(&mut pcm_buf);
                        if fc.has_timescale() {
                            encode_ready = fc.fill_frame(&mut ts_frame_buf);
                            use_ts = true;
                        }
                    }
                }

                if !encode_ready { continue; }

                // 4. Encode & Send
                let target_buf = if use_ts { &ts_frame_buf } else { &pcm_buf };

                let size = match encoder.encode(target_buf, &mut opus_buf) {
                    Ok(s) => s,
                    Err(e) => {
                        error!("Encoding failure: {}", e);
                        0
                    }
                };

                if size > 0 {
                    let mut d = dave.lock().await;
                    if let Ok(packet) = d.encrypt_opus(&opus_buf[..size]) {
                        let _ = udp.send_opus_packet(&packet).await;
                    }
                }
            }
        }
    }
    Ok(())
}
