use super::types::map_boxed_err;
use crate::{
    audio::{Mixer, engine::Encoder},
    common::types::{AnyResult, Shared},
    gateway::{DaveHandler, UdpBackend},
};
use std::{
    net::SocketAddr,
    sync::{Arc, atomic::Ordering},
};
use tokio_util::sync::CancellationToken;
use tracing::{error, warn};

pub async fn discover_ip(
    socket: &tokio::net::UdpSocket,
    addr: SocketAddr,
    ssrc: u32,
) -> AnyResult<(String, u16)> {
    let mut packet = [0u8; 74];
    packet[0..2].copy_from_slice(&1u16.to_be_bytes());
    packet[2..4].copy_from_slice(&70u16.to_be_bytes());
    packet[4..8].copy_from_slice(&ssrc.to_be_bytes());

    socket.send_to(&packet, addr).await.map_err(map_boxed_err)?;

    let mut buf = [0u8; 74];
    let timeout = tokio::time::Duration::from_secs(2);
    match tokio::time::timeout(timeout, socket.recv(&mut buf)).await {
        Ok(Ok(n)) => {
            if n < 74 {
                return Err(map_boxed_err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "IP discovery response too short",
                )));
            }
            let ip_str = std::str::from_utf8(&buf[8..72])
                .map_err(map_boxed_err)?
                .trim_matches('\0')
                .to_string();
            let port = u16::from_le_bytes([buf[72], buf[73]]);
            Ok((ip_str, port))
        }
        Ok(Err(e)) => Err(map_boxed_err(e)),
        Err(_) => Err(map_boxed_err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "IP discovery timeout",
        ))),
    }
}

#[allow(clippy::too_many_arguments)]
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
    let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(20));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let mut pcm_buf = vec![0i16; 1920];
    let mut opus_buf = vec![0u8; 4000];
    let mut silence_frames = 0;
    let mut ts_frame_buf = vec![0i16; 1920];

    loop {
        tokio::select! {
            _ = cancel_token.cancelled() => break,
            _ = interval.tick() => {
                let has_audio;
                {
                    let mut mixer_lock = mixer.lock().await;
                    if let Some(opus_frame) = mixer_lock.take_opus_frame() {
                        drop(mixer_lock);
                        silence_frames = 0;
                        frames_sent.fetch_add(1, Ordering::Relaxed);
                        let encrypted = dave.lock().await.encrypt_opus(&opus_frame);
                        match encrypted {
                            Ok(packet) => {
                                if let Err(e) = udp.send_opus_packet(&packet).await {
                                    warn!("Failed to send passthrough UDP packet: {}", e);
                                }
                            }
                            Err(e) => error!("DAVE passthrough encryption failed: {}", e),
                        }
                        continue;
                    }
                    has_audio = mixer_lock.mix(&mut pcm_buf);
                }

                if has_audio {
                    silence_frames = 0;
                    frames_sent.fetch_add(1, Ordering::Relaxed);
                } else {
                    silence_frames += 1;
                    frames_nulled.fetch_add(1, Ordering::Relaxed);
                    if silence_frames > 5 { continue; }
                }

                {
                    let mut fc = filter_chain.lock().await;
                    if fc.is_active() {
                        fc.process(&mut pcm_buf);
                        if fc.has_timescale() {
                            let timescale_frame_ready = fc.fill_frame(&mut ts_frame_buf);
                            drop(fc);
                            if timescale_frame_ready {
                                let size = encoder.encode(&ts_frame_buf, &mut opus_buf).map_err(map_boxed_err)?;
                                if size > 0 {
                                    let encrypted = dave.lock().await.encrypt_opus(&opus_buf[..size]);
                                    match encrypted {
                                        Ok(packet) => {
                                            if let Err(e) = udp.send_opus_packet(&packet).await {
                                                warn!("Failed to send UDP packet: {}", e);
                                            }
                                        }
                                        Err(e) => error!("DAVE encryption failed: {}", e),
                                    }
                                }
                            }
                            continue;
                        }
                    }
                }

                let size = encoder.encode(&pcm_buf, &mut opus_buf).map_err(map_boxed_err)?;
                if size > 0 {
                    let encrypted = dave.lock().await.encrypt_opus(&opus_buf[..size]);
                    match encrypted {
                        Ok(packet) => {
                            if let Err(e) = udp.send_opus_packet(&packet).await {
                                warn!("Failed to send UDP packet: {}", e);
                            }
                        }
                        Err(e) => error!("DAVE encryption failed: {}", e),
                    }
                }
            }
        }
    }
    Ok(())
}
