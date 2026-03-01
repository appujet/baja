//! Demux layer — format detection and container parsing.
//!
//! # Usage
//!
//! ```rust
//! use crate::audio::demux::{detect_format, AudioFormat};
//!
//! let header = &bytes[..12]; // first bytes of the stream
//! match detect_format(header) {
//!     AudioFormat::WebmOpus => { /* use WebmOpusDemuxer */ }
//!     _ => { /* use AudioProcessor transcode path */ }
//! }
//! ```

pub mod format;
pub mod webm_opus;

pub use format::detect_format;
use symphonia::core::{
    codecs::{CODEC_TYPE_NULL, Decoder, DecoderOptions},
    errors::Error,
    formats::{FormatOptions, FormatReader},
    io::{MediaSource, MediaSourceStream},
    meta::MetadataOptions,
    probe::Hint,
};
pub use webm_opus::WebmOpusDemuxer;

use crate::audio::constants::{MIXER_CHANNELS, TARGET_SAMPLE_RATE};
pub use crate::common::types::AudioFormat;

/// Resolved demux result returned by `open_format`.
pub enum DemuxResult {
    /// Symphonia-probed format reader + selected track id + codec decoder.
    Transcode {
        format: Box<dyn FormatReader>,
        track_id: u32,
        decoder: Box<dyn Decoder>,
        sample_rate: u32,
        channels: usize,
    },
}

/// Open a media source and detect its format.
///
/// Returns a `DemuxResult` describing the decode path, or a symphonia `Error`.
pub fn open_format(
    source: Box<dyn MediaSource>,
    kind: Option<crate::common::types::AudioFormat>,
) -> Result<DemuxResult, Error> {
    let mss = MediaSourceStream::new(source, Default::default());

    let mut hint = Hint::new();
    if let Some(k) = &kind {
        let ext = k.as_ext();
        if !ext.is_empty() {
            hint.with_extension(ext);
        }
        if let Some(mime) = k.as_mime() {
            hint.mime_type(mime);
        }
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
    let sample_rate = track.codec_params.sample_rate.unwrap_or(TARGET_SAMPLE_RATE);
    let channels = track
        .codec_params
        .channels
        .map(|c| c.count())
        .unwrap_or(MIXER_CHANNELS);

    // Build the right decoder for this codec.
    let decoder: Box<dyn Decoder> = if codec == symphonia::core::codecs::CODEC_TYPE_OPUS {
        Box::new(
            crate::audio::codec::opus_decoder::OpusCodecDecoder::try_new(
                &track.codec_params,
                &DecoderOptions::default(),
            )?,
        )
    } else {
        symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default())?
    };

    Ok(DemuxResult::Transcode {
        format,
        track_id,
        decoder,
        sample_rate,
        channels,
    })
}
