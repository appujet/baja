use std::sync::Arc;

use flume::{Receiver, Sender};
use symphonia::core::{
    audio::SampleBuffer,
    codecs::{CODEC_TYPE_NULL, CODEC_TYPE_OPUS, Decoder, DecoderOptions},
    errors::Error,
    formats::{FormatOptions, FormatReader},
    io::{MediaSource, MediaSourceStream},
    meta::MetadataOptions,
    probe::Hint,
};
use tracing::{Level, debug, info, span, warn};

use crate::audio::{
    buffer::{PooledBuffer, get_pool},
    pipeline::resampler::Resampler,
};

#[derive(Debug, Clone, PartialEq)]
pub enum DecoderCommand {
    Seek(u64), // Position in milliseconds
    Stop,
}

#[derive(Debug, PartialEq)]
pub enum CommandOutcome {
    Stop,
    Seeked,
    SeekFailed,
    None,
}

/// Audio processor that handles decoding and resampling.
///
/// ## Modes
///
/// **Passthrough** (`CODEC_TYPE_OPUS`): Reads raw Opus packet bytes from the
/// container and forwards them directly via `opus_tx`.  No decode, no resample,
/// no encode — the bytes go straight to Discord.
///
/// **Transcode** (all other codecs): Decodes to PCM i16, resamples to 48 kHz,
/// and sends `PooledBuffer` chunks via `pcm_tx`.
pub struct AudioProcessor {
    format: Box<dyn FormatReader>,
    decoder: Option<Box<dyn Decoder>>,
    resampler: Resampler,
    track_id: u32,
    /// PCM output channel — used in transcode mode.
    pcm_tx: Sender<PooledBuffer>,
    /// Raw Opus output channel — used in passthrough mode.
    opus_tx: Option<Sender<Arc<Vec<u8>>>>,
    cmd_rx: Receiver<DecoderCommand>,
    error_tx: Option<flume::Sender<String>>,
    sample_buf: Option<SampleBuffer<i16>>,
    passthrough: bool,

    // Audio specs
    source_rate: u32,
    target_rate: u32,
    channels: usize,
}

impl AudioProcessor {
    /// Create a new processor.
    ///
    /// - `pcm_tx`  — channel for decoded PCM samples (transcode path)
    /// - `opus_tx` — if `Some`, enables Opus passthrough for WebM/Opus streams
    pub fn new(
        source: Box<dyn MediaSource>,
        kind: Option<crate::common::types::AudioKind>,
        pcm_tx: Sender<PooledBuffer>,
        cmd_rx: Receiver<DecoderCommand>,
        error_tx: Option<flume::Sender<String>>,
    ) -> Result<Self, Error> {
        Self::new_with_passthrough(source, kind, pcm_tx, None, cmd_rx, error_tx)
    }

    /// Like `new`, but with an optional passthrough sender for raw Opus frames.
    pub fn new_with_passthrough(
        source: Box<dyn MediaSource>,
        kind: Option<crate::common::types::AudioKind>,
        pcm_tx: Sender<PooledBuffer>,
        opus_tx: Option<Sender<Arc<Vec<u8>>>>,
        cmd_rx: Receiver<DecoderCommand>,
        error_tx: Option<flume::Sender<String>>,
    ) -> Result<Self, Error> {
        let mss = MediaSourceStream::new(source, Default::default());
        let mut hint = Hint::new();

        if let Some(k) = kind {
            hint.with_extension(k.as_ext());
        }

        let probed = symphonia::default::get_probe().format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )?;

        let format = probed.format;
        let track = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or_else(|| {
                Error::IoError(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "no audio track found",
                ))
            })?;

        let track_id = track.id;
        let codec = track.codec_params.codec;

        // Enable passthrough when:
        //  1. The stream is Opus (already encoded to what Discord needs), AND
        //  2. A passthrough sender was provided (i.e. no active filters requiring PCM).
        let passthrough = codec == CODEC_TYPE_OPUS && opus_tx.is_some();

        let decoder: Option<Box<dyn Decoder>> = if passthrough {
            // Passthrough: we only read packets, never decode. No decoder needed.
            info!("AudioProcessor: OpusPassthrough mode — raw frames, zero transcode");
            None
        } else if codec == CODEC_TYPE_OPUS {
            // Opus codec but no passthrough tx (filters active) — use audiopus decoder.
            info!("AudioProcessor: Transcode mode (Opus → PCM, filters active)");
            Some(Box::new(
                crate::audio::codecs::opus::OpusCodecDecoder::try_new(
                    &track.codec_params,
                    &DecoderOptions::default(),
                )?,
            ))
        } else {
            info!("AudioProcessor: Transcode mode ({})", codec);
            Some(
                symphonia::default::get_codecs()
                    .make(&track.codec_params, &DecoderOptions::default())?,
            )
        };

        let source_rate = track.codec_params.sample_rate.unwrap_or(48000);
        let target_rate = 48000;
        let channels = track.codec_params.channels.map(|c| c.count()).unwrap_or(2);

        Ok(Self {
            format,
            decoder,
            resampler: Resampler::new(source_rate, target_rate, channels),
            track_id,
            pcm_tx,
            opus_tx,
            cmd_rx,
            error_tx,
            sample_buf: None,
            passthrough,
            source_rate,
            target_rate,
            channels,
        })
    }

    /// The main execution loop.
    pub fn run(&mut self) -> Result<(), Error> {
        let _span = span!(Level::DEBUG, "audio_processor").entered();

        if self.passthrough {
            info!(
                "OpusPassthrough loop: {}Hz {}ch (raw Opus → Discord)",
                self.source_rate, self.channels
            );
            return self.run_passthrough();
        }

        info!(
            "Starting transcode loop: {}Hz {}ch -> {}Hz",
            self.source_rate, self.channels, self.target_rate
        );
        self.run_transcode()
    }

    /// Passthrough mode: read raw Opus packets from the container and forward
    /// them directly.  Zero decode, zero resample, zero encode.
    fn run_passthrough(&mut self) -> Result<(), Error> {
        let opus_tx = match &self.opus_tx {
            Some(tx) => tx.clone(),
            None => return Ok(()),
        };

        loop {
            if self.check_commands() == CommandOutcome::Stop {
                break;
            }

            let packet = match self.format.next_packet() {
                Ok(p) => p,
                Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(e) => {
                    if let Some(tx) = &self.error_tx {
                        let _ = tx.send(format!("Packet read error: {}", e));
                    }
                    return Err(e);
                }
            };

            if packet.track_id() != self.track_id {
                continue;
            }

            // Forward raw Opus bytes — no decoding whatsoever.
            let frame = Arc::new(packet.data.to_vec());
            if opus_tx.send(frame).is_err() {
                break; // Mixer disconnected
            }
        }

        debug!("Passthrough loop finished");
        Ok(())
    }

    /// Transcode mode: decode packets to PCM, resample, and send PooledBuffers.
    fn run_transcode(&mut self) -> Result<(), Error> {
        let pool = get_pool();

        loop {
            if self.check_commands() == CommandOutcome::Stop {
                break;
            }

            let packet = match self.format.next_packet() {
                Ok(p) => p,
                Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(e) => {
                    if let Some(tx) = &self.error_tx {
                        let _ = tx.send(format!("Packet read error: {}", e));
                    }
                    return Err(e);
                }
            };

            if packet.track_id() != self.track_id {
                continue;
            }

            let decoder = match &mut self.decoder {
                Some(d) => d,
                None => break,
            };

            match decoder.decode(&packet) {
                Ok(decoded) => {
                    let spec = *decoded.spec();
                    let mut buf = match self.sample_buf.take() {
                        Some(b) => b,
                        None => SampleBuffer::<i16>::new(decoded.capacity() as u64, spec),
                    };

                    buf.copy_interleaved_ref(decoded);
                    let samples = buf.samples();

                    if !samples.is_empty() {
                        let mut pooled = pool.acquire();
                        if self.source_rate != self.target_rate {
                            self.resampler.process(samples, &mut pooled);
                        } else {
                            pooled.extend_from_slice(samples);
                        }

                        if !pooled.is_empty() {
                            if self.pcm_tx.send(pooled).is_err() {
                                return Ok(()); // Mixer disconnected
                            }
                        }
                    }
                    self.sample_buf = Some(buf);
                }
                Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(Error::DecodeError(e)) => {
                    warn!("Decode error (recoverable): {}", e);
                    continue;
                }
                Err(e) => {
                    if let Some(tx) = &self.error_tx {
                        let _ = tx.send(format!("Decode error: {}", e));
                    }
                    return Err(e);
                }
            }
        }

        debug!("Transcode loop finished");
        Ok(())
    }

    fn check_commands(&mut self) -> CommandOutcome {
        match self.cmd_rx.try_recv() {
            Ok(DecoderCommand::Seek(ms)) => {
                let time = symphonia::core::units::Time::from(ms as f64 / 1000.0);
                if let Ok(_) = self.format.seek(
                    symphonia::core::formats::SeekMode::Coarse,
                    symphonia::core::formats::SeekTo::Time {
                        time,
                        track_id: Some(self.track_id),
                    },
                ) {
                    self.resampler.reset();
                    if let Some(ref mut dec) = self.decoder {
                        dec.reset();
                    }
                    self.sample_buf = None;
                    CommandOutcome::Seeked
                } else {
                    warn!("AudioProcessor: seek to {}ms failed", ms);
                    CommandOutcome::SeekFailed
                }
            }
            Ok(DecoderCommand::Stop) | Err(flume::TryRecvError::Disconnected) => {
                CommandOutcome::Stop
            }
            _ => CommandOutcome::None,
        }
    }
}
