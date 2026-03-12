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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encoder_new() {
        let result = Encoder::new();
        assert!(result.is_ok(), "Encoder creation should succeed");
    }

    #[test]
    fn test_encoder_encode_silence() {
        let mut encoder = Encoder::new().unwrap();
        let input = vec![0i16; 960 * 2]; // 20ms stereo frame at 48kHz
        let mut output = vec![0u8; 4000];

        let result = encoder.encode(&input, &mut output);
        assert!(result.is_ok(), "Encoding silence should succeed");

        let size = result.unwrap();
        assert!(size > 0, "Encoded size should be greater than 0");
        assert!(
            size < output.len(),
            "Encoded size should be less than buffer"
        );
    }

    #[test]
    fn test_encoder_encode_tone() {
        let mut encoder = Encoder::new().unwrap();
        // Generate a simple 440Hz tone
        let mut input = vec![0i16; 960 * 2];
        for (i, sample) in input.iter_mut().enumerate() {
            let t = (i / 2) as f32 / 48000.0;
            *sample = (10000.0 * (2.0 * std::f32::consts::PI * 440.0 * t).sin()) as i16;
        }
        let mut output = vec![0u8; 4000];

        let result = encoder.encode(&input, &mut output);
        assert!(result.is_ok(), "Encoding tone should succeed");

        let size = result.unwrap();
        assert!(size > 0, "Encoded size should be greater than 0");
    }

    #[test]
    fn test_encoder_encode_max_amplitude() {
        let mut encoder = Encoder::new().unwrap();
        let input = vec![i16::MAX; 960 * 2];
        let mut output = vec![0u8; 4000];

        let result = encoder.encode(&input, &mut output);
        assert!(result.is_ok(), "Encoding max amplitude should succeed");
        assert!(result.unwrap() > 0);
    }

    #[test]
    fn test_encoder_encode_min_amplitude() {
        let mut encoder = Encoder::new().unwrap();
        let input = vec![i16::MIN; 960 * 2];
        let mut output = vec![0u8; 4000];

        let result = encoder.encode(&input, &mut output);
        assert!(result.is_ok(), "Encoding min amplitude should succeed");
        assert!(result.unwrap() > 0);
    }

    #[test]
    fn test_encoder_encode_multiple_frames() {
        let mut encoder = Encoder::new().unwrap();
        let input = vec![1000i16; 960 * 2];
        let mut output = vec![0u8; 4000];

        // Encode multiple frames to ensure state is maintained
        for _ in 0..5 {
            let result = encoder.encode(&input, &mut output);
            assert!(result.is_ok(), "Multiple encodings should succeed");
            assert!(result.unwrap() > 0);
        }
    }

    #[test]
    fn test_encoder_output_varies_with_input() {
        let mut encoder1 = Encoder::new().unwrap();
        let mut output1 = vec![0u8; 4000];

        // Silence
        let silence = vec![0i16; 960 * 2];
        let size1 = encoder1.encode(&silence, &mut output1).unwrap();

        let mut encoder2 = Encoder::new().unwrap();
        let mut output2 = vec![0u8; 4000];

        // Tone
        let tone = vec![5000i16; 960 * 2];
        let size2 = encoder2.encode(&tone, &mut output2).unwrap();

        assert!(size1 > 0);
        assert!(size2 > 0);
        assert_ne!(
            &output1[..size1],
            &output2[..size2],
            "Encoded silence and tone payloads should differ"
        );
    }
}
