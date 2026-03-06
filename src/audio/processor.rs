use std::io::ErrorKind;

use flume::Receiver;
use symphonia::core::{
    audio::SampleBuffer,
    codecs::Decoder,
    errors::Error,
    formats::{FormatReader, SeekMode, SeekTo},
    io::MediaSource,
    units::Time,
};
use tracing::{Level, debug, info, span, warn};

use crate::{
    audio::{
        buffer::PooledBuffer,
        constants::TARGET_SAMPLE_RATE,
        demux::{DemuxResult, open_format},
        engine::{BoxedEngine, TranscodeEngine},
        resample::Resampler,
    },
    common::types::AudioFormat,
    config::player::{PlayerConfig, ResamplingQuality},
};

#[derive(Debug, Clone, PartialEq)]
pub enum DecoderCommand {
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
    pub fn new(
        source: Box<dyn MediaSource>,
        kind: Option<AudioFormat>,
        pcm_tx: flume::Sender<PooledBuffer>,
        cmd_rx: Receiver<DecoderCommand>,
        error_tx: Option<flume::Sender<String>>,
        config: PlayerConfig,
    ) -> Result<Self, Error> {
        Self::with_engine(
            source,
            kind,
            Box::new(TranscodeEngine::new(pcm_tx)),
            cmd_rx,
            error_tx,
            config,
        )
    }

    pub fn with_engine(
        source: Box<dyn MediaSource>,
        kind: Option<AudioFormat>,
        engine: BoxedEngine,
        cmd_rx: Receiver<DecoderCommand>,
        error_tx: Option<flume::Sender<String>>,
        config: PlayerConfig,
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
            match config.resampling_quality {
                ResamplingQuality::Low => {
                    Resampler::linear(sample_rate, TARGET_SAMPLE_RATE, channels)
                }
                ResamplingQuality::Medium => {
                    Resampler::hermite(sample_rate, TARGET_SAMPLE_RATE, channels)
                }
                ResamplingQuality::High => {
                    Resampler::sinc(sample_rate, TARGET_SAMPLE_RATE, channels)
                }
            }
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
                Err(Error::IoError(e)) if e.kind() == ErrorKind::UnexpectedEof => break,
                Err(e) => {
                    self.send_error(format!("Packet read error: {e}"));
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
                        let mut pooled = Vec::with_capacity(samples.len());
                        if self.resampler.is_passthrough() {
                            pooled.extend_from_slice(samples);
                        } else {
                            self.resampler.process(samples, &mut pooled);
                        }

                        if !pooled.is_empty() && !self.engine.push_pcm(pooled) {
                            return Ok(());
                        }
                    }

                    self.sample_buf = Some(buf);
                }
                Err(Error::IoError(e)) if e.kind() == ErrorKind::UnexpectedEof => break,
                Err(Error::DecodeError(e)) => warn!("Decode error (recoverable): {e}"),
                Err(e) => {
                    self.send_error(format!("Decode error: {e}"));
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
                let time = Time::from(ms as f64 / 1000.0);
                if self
                    .format
                    .seek(
                        SeekMode::Coarse,
                        SeekTo::Time {
                            time,
                            track_id: Some(self.track_id),
                        },
                    )
                    .is_ok()
                {
                    self.resampler.reset();
                    self.decoder.reset();
                    self.sample_buf = None;
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

    fn send_error(&self, msg: String) {
        if let Some(tx) = &self.error_tx {
            let _ = tx.send(msg);
        }
    }
}
