use super::resampler::Resampler;
use crate::audio::hls_reader::HlsReader;
use crate::audio::reader::RemoteReader;
use crate::sources::youtube::sabr::reader::SabrReader;
use crate::sources::youtube::sabr::structs::FormatId;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use flume::{Receiver, Sender};
use serde::Deserialize;
use std::thread;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CODEC_TYPE_NULL, DecoderOptions};
use symphonia::core::errors::Error;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use tracing::{debug, error};

#[derive(Deserialize)]
struct SabrData {
    url: String,
    config: String,
    #[serde(rename = "clientName")]
    client_name: i32,
    #[serde(rename = "clientVersion")]
    client_version: String,
    #[serde(rename = "visitorData")]
    visitor_data: String,
    formats: Vec<FormatId>,
}

#[derive(Debug)]
pub enum DecoderCommand {
    Seek(u64), // Position in milliseconds
    Stop,
}

use crate::sources::youtube::cipher::YouTubeCipherManager;
use std::sync::Arc;

pub fn start_decoding(
    url: String,
    local_addr: Option<std::net::IpAddr>,
    cipher_manager: Option<Arc<YouTubeCipherManager>>,
    proxy: Option<crate::configs::HttpProxyConfig>,
) -> (Receiver<i16>, Sender<DecoderCommand>) {
    // Reduced buffer size for lower latency (was 512 * 1024)
    // 4096 * 4 samples @ 48kHz stereo ≈ 170ms
    let (tx, rx) = flume::bounded::<i16>(4096 * 4);
    let (cmd_tx, cmd_rx) = flume::unbounded::<DecoderCommand>();

    let handle = tokio::runtime::Handle::current();
    thread::spawn(move || {
        let _guard = handle.enter();
        if let Err(e) = decode_loop(url, local_addr, cipher_manager, tx, cmd_rx) {
            error!("Decoding error: {}", e);
        }
    });

    (rx, cmd_tx)
}

/// Returns `true` for URLs that point to an HLS manifest (`.m3u8`).
/// Covers YouTube's manifest URLs which use a path prefix without an extension,
/// and also plain `.m3u8` URLs from other sources.
fn is_hls_url(url: &str) -> bool {
    // Extension-based detection (plain HLS servers).
    if url.contains(".m3u8") {
        return true;
    }
    // YouTube HLS manifests have this path prefix.
    if url.contains("/api/manifest/hls_") {
        return true;
    }
    false
}

fn decode_loop(
    url: String,
    local_addr: Option<std::net::IpAddr>,
    cipher_manager: Option<Arc<YouTubeCipherManager>>,
    proxy: Option<crate::configs::HttpProxyConfig>,
    tx: Sender<i16>,
    rx_cmd: Receiver<DecoderCommand>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    debug!("Connecting to {}... (via {:?})", url, local_addr);

    let source: Box<dyn MediaSource> = if url.starts_with("sabr://") {
        let encoded = &url[7..];
        let decoded = BASE64_STANDARD.decode(encoded)?;
        let sabr_data: SabrData = serde_json::from_slice(&decoded)?;

        let config_bytes = if sabr_data.config.is_empty() {
            Vec::new()
        } else {
            BASE64_STANDARD.decode(sabr_data.config)?
        };

        let (reader, _handle) = SabrReader::new(
            sabr_data.url,
            config_bytes,
            sabr_data.client_name,
            sabr_data.client_version,
            sabr_data.visitor_data,
            sabr_data.formats,
        );
        Box::new(reader)
    } else if is_hls_url(&url) {
        debug!("HLS manifest detected, using HlsReader");
        // For YouTube HLS, we might need to resolve n-tokens.
        // We use the URL as a hint for the player URL if it's a YouTube manifest.
        let player_url = if url.contains("youtube.com") {
            Some(url.clone())
        } else {
            None
        };
        Box::new(HlsReader::new(
            &url,
            local_addr,
            cipher_manager,
            player_url,
        )?)
    } else {
        Box::new(RemoteReader::new(&url, local_addr)?)
    };

    let source = RemoteReader::new(&url, local_addr, proxy)?;
    debug!("Connected. Probing stream...");
    let mss = MediaSourceStream::new(source, Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = std::path::Path::new(&url)
        .extension()
        .and_then(|s| s.to_str())
    {
        match ext.to_lowercase().as_str() {
            "mp3" => {
                hint.with_extension("mp3");
            }
            "m4a" | "mp4" | "3gp" | "mov" => {
                hint.with_extension("m4a");
            }
            "ogg" | "opus" => {
                hint.with_extension("ogg");
            }
            "flac" => {
                hint.with_extension("flac");
            }
            "wav" => {
                hint.with_extension("wav");
            }
            "aac" => {
                hint.with_extension("aac");
            }
            "mkv" | "webm" => {
                hint.with_extension("mkv");
            }
            _ => {}
        }
    }
    debug!("Decoder hint: {:?}", hint);
    let probed = symphonia::default::get_probe().format(
        &hint,
        mss,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    )?;

    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or("no audio track found")?;

    let track_id = track.id;
    let mut decoder =
        symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default())?;

    let source_rate = track.codec_params.sample_rate.unwrap_or(48000);
    let target_rate = 48000;
    let channels = track.codec_params.channels.map(|c| c.count()).unwrap_or(2);

    debug!(
        "Source: {}Hz {} channels, Target: {}Hz",
        source_rate, channels, target_rate
    );

    let mut sample_buf = None;

    let mut resampler = Resampler::new(source_rate, target_rate, channels);

    loop {
        // Check for commands
        match rx_cmd.try_recv() {
            Ok(DecoderCommand::Seek(pos_ms)) => {
                debug!("Seeking to {}ms", pos_ms);
                let time = symphonia::core::units::Time::from(pos_ms as f64 / 1000.0);
                let mut seek_res = format.seek(
                    symphonia::core::formats::SeekMode::Accurate,
                    symphonia::core::formats::SeekTo::Time {
                        time: time.clone(),
                        track_id: Some(track_id),
                    },
                );

                if seek_res.is_err() {
                    debug!("Accurate seek failed, trying coarse seek");
                    seek_res = format.seek(
                        symphonia::core::formats::SeekMode::Coarse,
                        symphonia::core::formats::SeekTo::Time {
                            time,
                            track_id: Some(track_id),
                        },
                    );
                }

                match seek_res {
                    Ok(_) => {
                        debug!("Seek successful, resetting resampler/buffers");
                        // Resetting resampler ensures no old audio data is mixed in
                        resampler = Resampler::new(source_rate, target_rate, channels);
                        sample_buf = None; // Drop any pending samples
                        decoder.reset(); // Reset decoder state
                    }
                    Err(e) => {
                        error!("Seek failed: {}", e);
                    }
                }
            }
            Ok(DecoderCommand::Stop) => break,
            Err(flume::TryRecvError::Empty) => {}
            Err(flume::TryRecvError::Disconnected) => break,
        }

        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(Error::IoError(e)) => {
                if tx.is_disconnected() {
                    return Ok(());
                }
                return Err(Box::new(e));
            }
            Err(Error::DecodeError(e)) => {
                error!("decode error: {}", e);
                continue;
            }
            Err(_) => break,
        };

        if packet.track_id() != track_id {
            continue;
        }

        match decoder.decode(&packet) {
            Ok(audio_buf) => {
                let spec = *audio_buf.spec();
                let mut buf = match sample_buf {
                    Some(b) => b,
                    None => {
                        let duration = audio_buf.capacity() as u64;
                        SampleBuffer::<i16>::new(duration, spec)
                    }
                };

                buf.copy_interleaved_ref(audio_buf);
                let samples = buf.samples();

                if source_rate != target_rate {
                    resampler.process(samples, &tx)?;
                } else {
                    for &s in samples {
                        if tx.send(s).is_err() {
                            return Ok(());
                        }
                    }
                }
                sample_buf = Some(buf);
            }
            Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(Error::IoError(e)) => {
                if tx.is_disconnected() {
                    return Ok(());
                }
                return Err(Box::new(e));
            }
            Err(Error::DecodeError(e)) => {
                error!("decode error: {}", e);
                continue;
            }
            Err(_) => break,
        }
    }

    Ok(())
}
