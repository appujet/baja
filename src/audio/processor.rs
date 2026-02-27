//! `AudioProcessor` — ties source → demux → decode → resample → engine.
//!
//! The processor owns the decode loop and delegates all downstream routing to
//! whichever [`Engine`] implementation was injected at construction time:
//!
//! - [`TranscodeEngine`] — PCM → `FlowController` → Mixer
//! - [`PassthroughEngine`] — raw Opus → Mixer passthrough lane

use flume::Receiver;
use symphonia::core::{audio::SampleBuffer, codecs::Decoder, errors::Error, formats::FormatReader};
use tracing::{Level, debug, info, span, warn};

use crate::audio::{
    buffer::PooledBuffer,
    constants::TARGET_SAMPLE_RATE,
    demux::{DemuxResult, open_format},
    engine::{BoxedEngine, TranscodeEngine},
    resample::Resampler,
};

#[derive(Debug, Clone, PartialEq)]
pub enum DecoderCommand {
    /// Seek to the given position in milliseconds.
    Seek(u64),
    Stop,
}

#[derive(Debug, PartialEq)]
pub enum CommandOutcome {
    Stop,
    Seeked,
    SeekFailed,
    None,
}

/// Decodes any supported container to 48 kHz stereo PCM i16 and drives the
/// injected [`Engine`] with the resulting samples.
pub struct AudioProcessor {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    resampler: Resampler,
    track_id: u32,
    engine: BoxedEngine,
    cmd_rx: Receiver<DecoderCommand>,
    error_tx: Option<flume::Sender<String>>,
    sample_buf: Option<SampleBuffer<i16>>,
    source_rate: u32,
    channels: usize,
}

impl AudioProcessor {
    /// Open `source`, detect its format/codec, initialise the resampler and
    /// wire up a [`TranscodeEngine`] that pushes PCM onto `pcm_tx`.
    pub fn new(
        source: Box<dyn symphonia::core::io::MediaSource>,
        kind: Option<crate::common::types::AudioFormat>,
        pcm_tx: flume::Sender<PooledBuffer>,
        cmd_rx: Receiver<DecoderCommand>,
        error_tx: Option<flume::Sender<String>>,
    ) -> Result<Self, Error> {
        let engine: BoxedEngine = Box::new(TranscodeEngine::new(pcm_tx));
        Self::build(source, kind, engine, cmd_rx, error_tx)
    }

    /// Same as [`new`] but accepts a pre-built engine (e.g. `PassthroughEngine`).
    pub fn with_engine(
        source: Box<dyn symphonia::core::io::MediaSource>,
        kind: Option<crate::common::types::AudioFormat>,
        engine: BoxedEngine,
        cmd_rx: Receiver<DecoderCommand>,
        error_tx: Option<flume::Sender<String>>,
    ) -> Result<Self, Error> {
        Self::build(source, kind, engine, cmd_rx, error_tx)
    }

    fn build(
        source: Box<dyn symphonia::core::io::MediaSource>,
        kind: Option<crate::common::types::AudioFormat>,
        engine: BoxedEngine,
        cmd_rx: Receiver<DecoderCommand>,
        error_tx: Option<flume::Sender<String>>,
    ) -> Result<Self, Error> {
        let DemuxResult::Transcode {
            format,
            track_id,
            decoder,
            sample_rate,
            channels,
        } = open_format(source, kind)?;

        info!(
            "AudioProcessor: opened format — {}Hz {}ch",
            sample_rate, channels
        );

        let resampler = if sample_rate == TARGET_SAMPLE_RATE {
            Resampler::linear(sample_rate, TARGET_SAMPLE_RATE, channels)
        } else {
            Resampler::hermite(sample_rate, TARGET_SAMPLE_RATE, channels)
        };

        Ok(Self {
            format,
            decoder,
            resampler,
            track_id,
            engine,
            cmd_rx,
            error_tx,
            sample_buf: None,
            source_rate: sample_rate,
            channels,
        })
    }

    /// Run the decode loop until the stream ends naturally or a `Stop` command
    /// arrives.
    pub fn run(&mut self) -> Result<(), Error> {
        let _span = span!(Level::DEBUG, "audio_processor").entered();

        info!(
            "Starting transcode loop: {}Hz {}ch -> {}Hz",
            self.source_rate, self.channels, TARGET_SAMPLE_RATE
        );

        loop {
            if self.check_commands() == CommandOutcome::Stop {
                break;
            }

            let packet = match self.format.next_packet() {
                Ok(p) => p,
                Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(e) => {
                    if let Some(tx) = &self.error_tx {
                        let _ = tx.send(format!("Packet read error: {e}"));
                    }
                    return Err(e);
                }
            };

            if packet.track_id() != self.track_id {
                continue;
            }

            match self.decoder.decode(&packet) {
                Ok(decoded) => {
                    let spec = *decoded.spec();
                    let mut buf = self.sample_buf.take().unwrap_or_else(|| {
                        SampleBuffer::<i16>::new(decoded.capacity() as u64, spec)
                    });

                    buf.copy_interleaved_ref(decoded);
                    let samples = buf.samples();

                    if !samples.is_empty() {
                        let mut pooled: PooledBuffer = Vec::with_capacity(samples.len());
                        if self.resampler.is_passthrough() {
                            pooled.extend_from_slice(samples);
                        } else {
                            self.resampler.process(samples, &mut pooled);
                        }

                        if !pooled.is_empty() && !self.engine.push_pcm(pooled) {
                            return Ok(()); // Engine/Mixer disconnected — clean exit
                        }
                    }

                    self.sample_buf = Some(buf);
                }
                Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(Error::DecodeError(e)) => {
                    warn!("Decode error (recoverable): {e}");
                    continue;
                }
                Err(e) => {
                    if let Some(tx) = &self.error_tx {
                        let _ = tx.send(format!("Decode error: {e}"));
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
                if self
                    .format
                    .seek(
                        symphonia::core::formats::SeekMode::Coarse,
                        symphonia::core::formats::SeekTo::Time {
                            time,
                            track_id: Some(self.track_id),
                        },
                    )
                    .is_ok()
                {
                    self.resampler.reset();
                    self.decoder.reset();
                    self.sample_buf = None;
                    // Send a flush sentinel so the FlowController drops stale
                    // pre-seek audio from pending_pcm immediately.
                    let _ = self.engine.push_pcm(Vec::new());
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
