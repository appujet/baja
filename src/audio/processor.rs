use flume::{Receiver, Sender};
use symphonia::core::{
    audio::SampleBuffer,
    codecs::{CODEC_TYPE_NULL, Decoder, DecoderOptions},
    errors::Error,
    formats::{FormatOptions, FormatReader},
    io::{MediaSource, MediaSourceStream},
    meta::MetadataOptions,
    probe::Hint,
};
use tracing::{Level, debug, info, span, warn};

use crate::audio::pipeline::resampler::Resampler;

#[derive(Debug, Clone, PartialEq)]
pub enum DecoderCommand {
    Seek(u64), // Position in milliseconds
    Stop,
}

#[derive(Debug, PartialEq)]
pub enum CommandOutcome {
    Stop,
    Seeked,
    None,
}

/// Audio processor that handles decoding and resampling.
pub struct AudioProcessor {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    resampler: Resampler,
    track_id: u32,
    tx: Sender<Vec<i16>>,
    cmd_rx: Receiver<DecoderCommand>,
    /// When set, fatal mid-playback errors are forwarded here so the
    /// playback loop can emit a `TrackExceptionEvent` instead of a
    /// silent `TrackEnd(Finished)`.
    error_tx: Option<flume::Sender<String>>,
    sample_buf: Option<SampleBuffer<i16>>,
    /// Scratch buffer for resampler output — reused each packet to avoid
    /// allocating a new Vec per decoded frame (H1 fix).
    resample_buf: Vec<i16>,

    // Audio specs
    source_rate: u32,
    target_rate: u32,
    channels: usize,
}

impl AudioProcessor {
    /// Initializes the processor by probing the source and setting up the codec.
    pub fn new(
        source: Box<dyn MediaSource>,
        kind: Option<crate::common::types::AudioKind>,
        tx: Sender<Vec<i16>>,
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
        let decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &DecoderOptions::default())?;

        let source_rate = track.codec_params.sample_rate.unwrap_or(48000);
        let target_rate = 48000;
        let channels = track.codec_params.channels.map(|c| c.count()).unwrap_or(2);

        Ok(Self {
            format,
            decoder,
            resampler: Resampler::new(source_rate, target_rate, channels),
            track_id,
            tx,
            cmd_rx,
            error_tx,
            sample_buf: None,
            resample_buf: Vec::with_capacity(4096), // pre-allocated; reused each packet
            source_rate,
            target_rate,
            channels,
        })
    }

    /// The main execution loop.
    pub fn run(&mut self) -> Result<(), Error> {
        let _span = span!(Level::DEBUG, "audio_processor").entered();
        info!(
            "Starting playback loop: {}Hz {}ch -> {}Hz",
            self.source_rate, self.channels, self.target_rate
        );

        loop {
            // 1. Handle External Commands
            if self.check_commands() == CommandOutcome::Stop {
                break;
            }

            // 2. Fetch Next Packet
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

            // 3. Decode Packet
            match self.decoder.decode(&packet) {
                Ok(decoded) => {
                    let spec = *decoded.spec();
                    let mut buf = match self.sample_buf.take() {
                        Some(b) => b,
                        None => SampleBuffer::<i16>::new(decoded.capacity() as u64, spec),
                    };

                    buf.copy_interleaved_ref(decoded);
                    let samples = buf.samples();

                    if self.source_rate != self.target_rate {
                        // H1: resampler fills resample_buf.
                        // H2: send the whole buffer as one Vec<i16> — one channel op per packet.
                        self.resample_buf.clear();
                        self.resampler.process(samples, &mut self.resample_buf);
                        if !self.resample_buf.is_empty() {
                            if self.tx.send(self.resample_buf.clone()).is_err() {
                                return Ok(()); // Mixer disconnected
                            }
                        }
                    } else {
                        // H2: one channel op per decoded packet instead of per sample.
                        if !samples.is_empty() && self.tx.send(samples.to_vec()).is_err() {
                            return Ok(()); // Mixer disconnected
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

        debug!("Playback loop finished");
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
                    self.resampler.reset(); // zero in-place — no heap allocation on seek
                    self.decoder.reset();
                    self.sample_buf = None;
                }
                CommandOutcome::Seeked
            }
            Ok(DecoderCommand::Stop) | Err(flume::TryRecvError::Disconnected) => {
                CommandOutcome::Stop
            }
            _ => CommandOutcome::None,
        }
    }
}
