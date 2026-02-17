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
        input: &[f32],
        output: &mut [u8],
    ) -> Result<usize, Box<dyn std::error::Error>> {
        let size = self.encoder.encode_float(input, output)?;
        Ok(size)
    }
}
