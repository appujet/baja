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
use tracing::{Level, debug, span, warn};

use crate::{
    audio::{
        AudioFrame,
        constants::{MIXER_CHANNELS, TARGET_SAMPLE_RATE},
        demux::{DemuxResult, open_format},
        engine::{BoxedEngine, StandardEngine},
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
    config: PlayerConfig,
    recoverable_errors: u32,
    downmix_buf: Vec<i16>,
}

impl AudioProcessor {
    pub fn new(
        source: Box<dyn MediaSource>,
        kind: Option<AudioFormat>,
        frame_tx: flume::Sender<AudioFrame>,
        cmd_rx: Receiver<DecoderCommand>,
        error_tx: Option<flume::Sender<String>>,
        config: PlayerConfig,
    ) -> Result<Self, Error> {
        Self::with_engine(
            source,
            kind,
            Box::new(StandardEngine::new(frame_tx)),
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

        debug!(
            "AudioProcessor: opened format — {}Hz {}ch",
            sample_rate, channels
        );

        let resampler = if sample_rate == TARGET_SAMPLE_RATE {
            Resampler::linear(sample_rate, TARGET_SAMPLE_RATE, MIXER_CHANNELS)
        } else {
            match config.resampling_quality {
                ResamplingQuality::Low => {
                    Resampler::linear(sample_rate, TARGET_SAMPLE_RATE, MIXER_CHANNELS)
                }
                ResamplingQuality::Medium => {
                    Resampler::hermite(sample_rate, TARGET_SAMPLE_RATE, MIXER_CHANNELS)
                }
                ResamplingQuality::High => {
                    Resampler::sinc(sample_rate, TARGET_SAMPLE_RATE, MIXER_CHANNELS)
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
            config,
            recoverable_errors: 0,
            downmix_buf: Vec::with_capacity(1920),
        })
    }

    pub fn run(&mut self) -> Result<(), Error> {
        let _span = span!(Level::DEBUG, "audio_processor").entered();

        debug!(
            "Starting transcode loop: {}Hz {}ch -> {}Hz",
            self.source_rate, self.channels, TARGET_SAMPLE_RATE
        );

        let mut packet_count = 0u64;
        loop {
            packet_count += 1;
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
                    self.recoverable_errors = 0;
                    let spec = *decoded.spec();
                    let mut buf = self.sample_buf.take().unwrap_or_else(|| {
                        SampleBuffer::<i16>::new(decoded.capacity() as u64, spec)
                    });

                    buf.copy_interleaved_ref(decoded);
                    let samples = buf.samples();

                    if !samples.is_empty() {
                        let frame_channels = spec.channels.count();
                        let frame_rate = spec.rate;

                        if frame_rate != self.source_rate {
                            debug!(
                                "AudioProcessor: frame rate mismatch ({}Hz vs {}Hz) — re-initializing resampler",
                                frame_rate, self.source_rate
                            );
                            self.source_rate = frame_rate;
                            self.resampler = if self.source_rate == TARGET_SAMPLE_RATE {
                                Resampler::linear(
                                    self.source_rate,
                                    TARGET_SAMPLE_RATE,
                                    MIXER_CHANNELS,
                                )
                            } else {
                                match self.config.resampling_quality {
                                    ResamplingQuality::Low => Resampler::linear(
                                        self.source_rate,
                                        TARGET_SAMPLE_RATE,
                                        MIXER_CHANNELS,
                                    ),
                                    ResamplingQuality::Medium => Resampler::hermite(
                                        self.source_rate,
                                        TARGET_SAMPLE_RATE,
                                        MIXER_CHANNELS,
                                    ),
                                    ResamplingQuality::High => Resampler::sinc(
                                        self.source_rate,
                                        TARGET_SAMPLE_RATE,
                                        MIXER_CHANNELS,
                                    ),
                                }
                            };
                        }

                        let pcm_data = if frame_channels == MIXER_CHANNELS {
                            samples
                        } else {
                            if packet_count.is_multiple_of(100) {
                                debug!(
                                    "AudioProcessor: Downmixing {}ch -> {}ch (samples: {})",
                                    frame_channels,
                                    MIXER_CHANNELS,
                                    samples.len()
                                );
                            }
                            let num_frames = samples.len() / frame_channels;
                            self.downmix_buf.clear();
                            self.downmix_buf.reserve(num_frames * MIXER_CHANNELS);

                            for i in 0..num_frames {
                                let frame = &samples[i * frame_channels..(i + 1) * frame_channels];
                                let mut l = 0i32;
                                let mut r = 0i32;

                                for (ch, &sample) in frame.iter().enumerate() {
                                    if ch % 2 == 0 {
                                        l += sample as i32;
                                    } else {
                                        r += sample as i32;
                                    }
                                }

                                let left_count = frame_channels.div_ceil(2);
                                let right_count = frame_channels / 2;

                                self.downmix_buf.push((l / left_count as i32) as i16);
                                if right_count > 0 {
                                    self.downmix_buf.push((r / right_count as i32) as i16);
                                } else {
                                    // Upmix mono to stereo
                                    self.downmix_buf.push((l / left_count as i32) as i16);
                                }
                            }
                            &self.downmix_buf[..]
                        };

                        let capacity = (pcm_data.len() as f64 * TARGET_SAMPLE_RATE as f64
                            / self.source_rate as f64)
                            .ceil() as usize
                            + 32;
                        let mut resampled = crate::audio::buffer::acquire_buffer(capacity);
                        if self.resampler.is_passthrough() {
                            resampled.extend_from_slice(pcm_data);
                        } else {
                            self.resampler.process(pcm_data, &mut resampled);
                        }

                        if !resampled.is_empty() && !self.engine.push(AudioFrame::Pcm(resampled)) {
                            return Ok(());
                        }
                    }

                    self.sample_buf = Some(buf);
                }
                Err(Error::IoError(e)) if e.kind() == ErrorKind::UnexpectedEof => break,
                Err(Error::DecodeError(e)) => {
                    self.recoverable_errors += 1;
                    if self.recoverable_errors == 1 {
                        warn!("Decode error (recoverable): {e}");
                    } else if self.recoverable_errors.is_multiple_of(100) {
                        warn!(
                            "Decode error (recoverable, x{}): {e}",
                            self.recoverable_errors
                        );
                    }
                }
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
                    let _ = self.engine.push(AudioFrame::Pcm(Vec::new()));
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
