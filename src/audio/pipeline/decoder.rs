use super::resampler::Resampler;
use crate::audio::reader::RemoteReader;
use flume::{Receiver, Sender};
use std::thread;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use tracing::{debug, error};

pub fn start_decoding(url: String) -> Receiver<i16> {
    let (tx, rx) = flume::bounded::<i16>(512 * 1024);

    thread::spawn(move || {
        if let Err(e) = decode_loop(url, tx) {
            error!("Decoding error: {}", e);
        }
    });

    rx
}

fn decode_loop(url: String, tx: Sender<i16>) -> Result<(), Box<dyn std::error::Error>> {
    debug!("Connecting to {}...", url);
    let source = RemoteReader::new(&url)?;
    debug!("Connected. Probing stream...");
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

    debug!(
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
