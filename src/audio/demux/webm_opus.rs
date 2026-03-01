//! WebM / Matroska → raw Opus packet demuxer.
//!
//! Uses symphonia's built-in WebM/Matroska reader under the hood and exposes
//! a simple iterator that yields raw Opus byte slices, ready to be sent
//! directly to Discord without re-encoding.

use symphonia::core::{
    codecs::CODEC_TYPE_OPUS,
    errors::Error,
    formats::{FormatOptions, FormatReader},
    io::{MediaSource, MediaSourceStream},
    meta::MetadataOptions,
    probe::Hint,
};

/// A thin wrapper around symphonia's WebM reader that yields raw Opus packets.
pub struct WebmOpusDemuxer {
    format: Box<dyn FormatReader>,
    track_id: u32,
}

impl WebmOpusDemuxer {
    /// Open a WebM/Matroska source.  Returns `None` if no Opus track is found.
    pub fn open(source: Box<dyn MediaSource>) -> Result<Option<Self>, Error> {
        let mss = MediaSourceStream::new(source, Default::default());
        let mut hint = Hint::new();
        hint.with_extension("webm");

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
            .find(|t| t.codec_params.codec == CODEC_TYPE_OPUS);

        match track {
            Some(t) => {
                let track_id = t.id;
                Ok(Some(Self { format, track_id }))
            }
            None => Ok(None),
        }
    }

    /// Read the next raw Opus packet.
    ///
    /// Returns `Ok(None)` at end-of-stream and `Err(_)` on hard errors.
    pub fn next_packet(&mut self) -> Result<Option<Vec<u8>>, Error> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(p) => p,
                Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    return Ok(None);
                }
                Err(e) => return Err(e),
            };

            if packet.track_id() != self.track_id {
                continue; // skip non-Opus tracks (video, metadata, etc.)
            }

            return Ok(Some(packet.data.to_vec()));
        }
    }
}
