//! Opus encoder — wraps `audiopus` for encoding mixed PCM into Opus packets
//! sent over Discord UDP.
//!
//! Previously lived in `audio/pipeline/encoder.rs`; moved here so that
//! `audio/pipeline/` can be removed entirely.

use audiopus::{Application, Bitrate, Channels, SampleRate, coder::Encoder as OpusEncoder};

pub struct Encoder {
    encoder: OpusEncoder,
}

impl Encoder {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let mut encoder =
            OpusEncoder::new(SampleRate::Hz48000, Channels::Stereo, Application::Audio)?;
        encoder.set_bitrate(Bitrate::Auto)?;
        Ok(Self { encoder })
    }

    pub fn encode(
        &mut self,
        input: &[i16],
        output: &mut [u8],
    ) -> Result<usize, Box<dyn std::error::Error>> {
        let size = self.encoder.encode(input, output)?;
        Ok(size)
    }
}
