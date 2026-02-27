use audiopus::{Application, Channels, SampleRate, coder::Encoder as OpusEncoder};

/// PCM i16 → Opus bytes encoder.
/// Encodes 960-sample (20 ms) stereo frames at 48 kHz.
pub struct OpusCodecEncoder {
    encoder: OpusEncoder,
}

impl OpusCodecEncoder {
    /// Create a new encoder at 48 kHz stereo with the AUDIO application profile.
    pub fn new() -> Result<Self, audiopus::Error> {
        let encoder = OpusEncoder::new(SampleRate::Hz48000, Channels::Stereo, Application::Audio)?;
        Ok(Self { encoder })
    }

    /// Encode a 960-sample (per channel) interleaved i16 PCM slice into `out`.
    /// Returns the number of bytes written.
    pub fn encode(&mut self, pcm: &[i16], out: &mut [u8]) -> Result<usize, audiopus::Error> {
        self.encoder.encode(pcm, out)
    }
}
