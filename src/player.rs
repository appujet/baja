use std::io::{Read, Seek, SeekFrom};
use std::thread;

use flume::{Receiver, Sender};
use songbird::input::{Input, RawAdapter};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CODEC_TYPE_NULL, DecoderOptions};
use symphonia::core::errors::Error;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::source::HttpSource;
use tracing::{error, info};

pub fn play_url(url: String) -> Input {
    let (tx, rx) = flume::bounded::<f32>(1024 * 100);

    // Spawn the decoding thread
    thread::spawn(move || {
        if let Err(e) = decode_loop(url, tx) {
            eprintln!("Decoding error: {}", e);
        }
    });

    let reader = PlayerReader { rx };

    // Songbird 0.5 uses RawAdapter to wrap f32 PCM into a Symphonia-compatible stream
    let adapter = RawAdapter::new(reader, 48000, 2);
    Input::from(adapter)
}

struct PlayerReader {
    rx: Receiver<f32>,
}

impl Read for PlayerReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut written = 0;
        let mut chunks = buf.chunks_exact_mut(4);

        for chunk in chunks.by_ref() {
            match self.rx.recv_timeout(std::time::Duration::from_millis(2)) {
                Ok(sample) => {
                    let bytes = sample.to_le_bytes();
                    chunk.copy_from_slice(&bytes);
                    written += 4;
                }
                Err(flume::RecvTimeoutError::Timeout) => {
                    // Fill remaining buffer with silence to keep mixer timing
                    for rest in chunk {
                        *rest = 0;
                    }
                    for rest_chunk in chunks {
                        for byte in rest_chunk {
                            *byte = 0;
                        }
                    }
                    return Ok(buf.len());
                }
                Err(flume::RecvTimeoutError::Disconnected) => return Ok(written),
            }
        }

        Ok(written)
    }
}

impl Seek for PlayerReader {
    fn seek(&mut self, _pos: SeekFrom) -> std::io::Result<u64> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Cannot seek streaming source",
        ))
    }
}

impl MediaSource for PlayerReader {
    fn is_seekable(&self) -> bool {
        false
    }

    fn byte_len(&self) -> Option<u64> {
        None
    }
}

// Needed for Songbird Input
unsafe impl Send for PlayerReader {}
unsafe impl Sync for PlayerReader {}

fn decode_loop(url: String, tx: Sender<f32>) -> Result<(), Box<dyn std::error::Error>> {
    info!("Connecting to {}...", url);
    let source = HttpSource::new(&url)?;
    info!("Connected. Probing stream...");
    let mss = MediaSourceStream::new(Box::new(source), Default::default());

    let mut hint = Hint::new();
    if url.ends_with(".mp4") {
        hint.with_extension("mp4");
    } else if url.ends_with(".mp3") {
        hint.with_extension("mp3");
    }

    let format_opts = FormatOptions::default();
    let metadata_opts = MetadataOptions::default();
    let decoder_opts = DecoderOptions::default();

    let probed =
        symphonia::default::get_probe().format(&hint, mss, &format_opts, &metadata_opts)?;

    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or("no supported audio tracks")?;

    let mut decoder = symphonia::default::get_codecs().make(&track.codec_params, &decoder_opts)?;

    let track_id = track.id;
    let sample_rate = track
        .codec_params
        .sample_rate
        .ok_or("unknown sample rate")?;
    let channels = track
        .codec_params
        .channels
        .map(|c| c.count())
        .unwrap_or(2)
        .max(1);
    let target_rate = 48000;

    let mut sample_buf = None;

    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(Error::IoError(e)) => return Err(Box::new(e)),
            Err(Error::DecodeError(e)) => {
                eprintln!("decode error: {}", e);
                continue;
            }
            Err(_) => break, // EOF
        };

        if packet.track_id() != track_id {
            continue;
        }

        if packet.ts() % 1000 == 0 {
            info!("Decoded packet ts={}", packet.ts());
        }

        match decoder.decode(&packet) {
            Ok(audio_buf) => {
                let spec = *audio_buf.spec();
                let mut buf = match sample_buf {
                    Some(b) => b,
                    None => {
                        let duration = audio_buf.capacity() as u64;
                        SampleBuffer::new(duration, spec)
                    }
                };
                buf.copy_interleaved_ref(audio_buf);

                let source_rate = sample_rate;
                let samples = buf.samples();

                if source_rate == target_rate {
                    for &sample in samples {
                        if tx.send(sample).is_err() {
                            return Ok(());
                        }
                    }
                } else {
                    let ratio = source_rate as f32 / target_rate as f32;
                    let mut src_idx = 0.0;
                    let frame_count = samples.len() / channels;

                    while (src_idx as usize) < frame_count - 1 {
                        let idx = src_idx as usize;
                        let frac = src_idx - idx as f32;

                        for c in 0..channels {
                            let s1 = samples[idx * channels + c];
                            let s2 = samples[(idx + 1) * channels + c];
                            let interpolated = s1 + (s2 - s1) * frac;
                            if tx.send(interpolated).is_err() {
                                return Ok(());
                            }
                        }
                        src_idx += ratio;
                    }
                }
                sample_buf = Some(buf);
            }
            Err(Error::IoError(e)) => return Err(Box::new(e)),
            Err(Error::DecodeError(e)) => {
                eprintln!("decode error: {}", e);
                continue;
            }
            Err(_) => break,
        }
    }

    Ok(())
}
