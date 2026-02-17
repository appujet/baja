use std::thread;

use flume::{Receiver, Sender};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CODEC_TYPE_NULL, DecoderOptions};
use symphonia::core::errors::Error;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::source::HttpSource;
use tracing::{error, info};

struct Resampler {
    ratio: f64,
    index: f64,
    last_samples: Vec<i16>,
    channels: usize,
}

impl Resampler {
    fn new(source_rate: u32, target_rate: u32, channels: usize) -> Self {
        Self {
            ratio: source_rate as f64 / target_rate as f64,
            index: 0.0,
            last_samples: vec![0; channels],
            channels,
        }
    }

    fn process(
        &mut self,
        input: &[i16],
        tx: &Sender<i16>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let num_frames = input.len() / self.channels;

        while self.index < num_frames as f64 {
            let idx = self.index as usize;
            let fract = self.index.fract();

            for c in 0..self.channels {
                let s1 = if idx == 0 {
                    self.last_samples[c] as f64
                } else {
                    input[(idx - 1) * self.channels + c] as f64
                };

                let s2 = if idx < num_frames {
                    input[idx * self.channels + c] as f64
                } else {
                    // Should not happen with while condition, but safety check
                    input[(num_frames - 1) * self.channels + c] as f64
                };

                // Linear interpolation on i16
                let s = s1 * (1.0 - fract) + s2 * fract;

                if tx.send(s as i16).is_err() {
                    return Ok(());
                }
            }

            self.index += self.ratio;
        }

        // Update index relative to the new buffer start
        self.index -= num_frames as f64;

        // Store last samples for next chunk's start
        if num_frames > 0 {
            for c in 0..self.channels {
                self.last_samples[c] = input[(num_frames - 1) * self.channels + c];
            }
        }

        Ok(())
    }
}

pub fn start_decoding(url: String) -> Receiver<i16> {
    let (tx, rx) = flume::bounded::<i16>(512 * 1024);

    // Spawn the decoding thread
    thread::spawn(move || {
        if let Err(e) = decode_loop(url, tx) {
            error!("Decoding error: {}", e);
        }
    });

    rx
}

fn decode_loop(url: String, tx: Sender<i16>) -> Result<(), Box<dyn std::error::Error>> {
    tracing::debug!("Connecting to {}...", url);
    let source = HttpSource::new(&url)?;
    tracing::debug!("Connected. Probing stream...");
    let mss = MediaSourceStream::new(Box::new(source), Default::default());

    let mut hint = Hint::new();
    if url.ends_with(".mp4") {
        hint.with_extension("mp4");
    }

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

    tracing::debug!(
        "Source: {}Hz {} channels, Target: {}Hz",
        source_rate, channels, target_rate
    );

    let mut sample_buf = None;
    let mut resampler = Resampler::new(source_rate, target_rate, channels);

    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(Error::IoError(e)) => return Err(Box::new(e)),
            Err(Error::DecodeError(e)) => {
                error!("decode error: {}", e);
                continue;
            }
            Err(_) => break, // EOF
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
            Err(Error::IoError(e)) => return Err(Box::new(e)),
            Err(Error::DecodeError(e)) => {
                error!("decode error: {}", e);
                continue;
            }
            Err(_) => break,
        }
    }

    Ok(())
}
