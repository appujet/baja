use audiopus::{Application, Bitrate, Channels, SampleRate, coder::Encoder as OpusEncoder};

use crate::common::types::AnyResult;

pub struct Encoder {
    encoder: OpusEncoder,
}

impl Encoder {
    /// Creates a new `Encoder` wrapping an Opus encoder configured for 48 kHz stereo audio.
    ///
    /// The returned `Encoder` contains an Opus encoder with Application::Audio and an automatic bitrate setting.
    ///
    /// # Examples
    ///
    /// ```
    /// let enc = crate::audio::engine::encoder::Encoder::new().unwrap();
    /// // `enc` is ready to encode 48 kHz stereo audio frames.
    /// ```
    pub fn new() -> AnyResult<Self> {
        let mut encoder =
            OpusEncoder::new(SampleRate::Hz48000, Channels::Stereo, Application::Audio)
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        encoder
            .set_bitrate(Bitrate::Auto)
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        Ok(Self { encoder })
    }

    /// Encodes PCM 16-bit samples into Opus-encoded bytes.
    ///
    /// # Examples
    ///
    /// ```
    /// use crate::audio::engine::encoder::Encoder;
    ///
    /// let mut enc = Encoder::new().unwrap();
    /// let input = vec![0i16; 960]; // 20ms of silence at 48kHz mono/stereo frame size
    /// let mut output = vec![0u8; 4000];
    /// let written = enc.encode(&input, &mut output).unwrap();
    /// assert!(written <= output.len());
    /// ```
    ///
    /// # Returns
    ///
    /// `usize` number of bytes written into `output`.
    pub fn encode(&mut self, input: &[i16], output: &mut [u8]) -> AnyResult<usize> {
        let size = self
            .encoder
            .encode(input, output)
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        Ok(size)
    }
}
