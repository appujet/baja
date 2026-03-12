use audiopus::{Application, Bitrate, Channels, SampleRate, coder::Encoder as OpusEncoder};

use crate::common::types::AnyResult;

pub struct Encoder {
    encoder: OpusEncoder,
}

impl Encoder {
    pub fn new() -> AnyResult<Self> {
        let mut encoder =
            OpusEncoder::new(SampleRate::Hz48000, Channels::Stereo, Application::Audio)
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        encoder
            .set_bitrate(Bitrate::Auto)
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        Ok(Self { encoder })
    }

    pub fn encode(&mut self, input: &[i16], output: &mut [u8]) -> AnyResult<usize> {
        let size = self
            .encoder
            .encode(input, output)
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        Ok(size)
    }
}
