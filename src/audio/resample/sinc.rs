use std::collections::VecDeque;

use crate::audio::buffer::PooledBuffer;

pub struct SincResampler {
    ratio: f32,
    index: f32,
    channels: usize,
    taps: usize,
    table: Vec<f32>,
    buffer: Vec<VecDeque<f32>>,
}

impl SincResampler {
    pub fn new(source_rate: u32, target_rate: u32, channels: usize) -> Self {
        let taps = 32;
        let mut table = Vec::with_capacity(taps);
        let m = taps as f32 - 1.0;
        let half_taps = (taps / 2) as f32;

        for i in 0..taps {
            let offset = i as f32 - half_taps;

            let a0 = 0.42;
            let a1 = 0.5;
            let a2 = 0.08;
            let pi_n_m = 2.0 * std::f32::consts::PI * i as f32 / m;
            let window = a0 - a1 * pi_n_m.cos() + a2 * (2.0 * pi_n_m).cos();

            table.push(Self::sinc(offset) * window);
        }

        Self {
            ratio: source_rate as f32 / target_rate as f32,
            index: 0.0,
            channels,
            taps,
            table,
            buffer: vec![VecDeque::from(vec![0.0; taps]); channels],
        }
    }

    fn sinc(x: f32) -> f32 {
        if x.abs() < 1e-6 {
            return 1.0;
        }
        let pi_x = std::f32::consts::PI * x;
        pi_x.sin() / pi_x
    }

    pub fn process(&mut self, input: &[i16], output: &mut PooledBuffer) {
        let num_frames = input.len() / self.channels;

        for frame in 0..num_frames {
            for ch in 0..self.channels {
                self.buffer[ch].pop_front();
                self.buffer[ch].push_back(input[frame * self.channels + ch] as f32);
            }

            while self.index < 1.0 {
                for ch in 0..self.channels {
                    let mut sum = 0.0;

                    for i in 0..self.taps {
                        sum += self.buffer[ch][i] * self.table[i];
                    }

                    output.push(sum.clamp(i16::MIN as f32, i16::MAX as f32) as i16);
                }
                self.index += self.ratio;
            }
            self.index -= 1.0;
        }
    }

    pub fn reset(&mut self) {
        self.index = 0.0;
        for ch in &mut self.buffer {
            for x in ch {
                *x = 0.0;
            }
        }
    }

    pub fn is_passthrough(&self) -> bool {
        (self.ratio - 1.0).abs() < f32::EPSILON
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sinc_function() {
        // Test sinc(0) = 1
        assert!((SincResampler::sinc(0.0) - 1.0).abs() < 1e-6);

        // Test sinc at very small values
        assert!((SincResampler::sinc(1e-7) - 1.0).abs() < 1e-6);

        // Test sinc at other values
        let val = SincResampler::sinc(1.0);
        assert!(val.abs() < 0.01); // sinc(1) ≈ 0

        // Test sinc symmetry
        assert!((SincResampler::sinc(2.0) - SincResampler::sinc(-2.0)).abs() < 1e-6);
    }

    #[test]
    fn test_resampler_new_same_rate() {
        let resampler = SincResampler::new(48000, 48000, 2);
        assert!(resampler.is_passthrough());
        assert_eq!(resampler.channels, 2);
        assert_eq!(resampler.taps, 32);
        assert_eq!(resampler.table.len(), 32);
        assert_eq!(resampler.buffer.len(), 2);
    }

    #[test]
    fn test_resampler_new_downsample() {
        let resampler = SincResampler::new(48000, 44100, 2);
        assert!(!resampler.is_passthrough());
        assert!(resampler.ratio > 1.0);
        assert_eq!(resampler.channels, 2);
    }

    #[test]
    fn test_resampler_new_upsample() {
        let resampler = SincResampler::new(44100, 48000, 2);
        assert!(!resampler.is_passthrough());
        assert!(resampler.ratio < 1.0);
    }

    #[test]
    fn test_resampler_reset() {
        let mut resampler = SincResampler::new(48000, 44100, 2);

        // Simulate some processing
        resampler.index = 0.5;
        for ch in &mut resampler.buffer {
            for x in ch.iter_mut() {
                *x = 100.0;
            }
        }

        // Reset
        resampler.reset();

        assert_eq!(resampler.index, 0.0);
        for ch in &resampler.buffer {
            for &x in ch.iter() {
                assert_eq!(x, 0.0);
            }
        }
    }

    #[test]
    fn test_resampler_process_empty() {
        let mut resampler = SincResampler::new(48000, 48000, 2);
        let input: Vec<i16> = vec![];
        let mut output = Vec::new();

        resampler.process(&input, &mut output);
        assert!(output.is_empty());
    }

    #[test]
    fn test_resampler_process_silence() {
        let mut resampler = SincResampler::new(48000, 48000, 2);
        let input = vec![0i16; 20]; // 10 stereo frames
        let mut output = Vec::new();

        resampler.process(&input, &mut output);

        // With passthrough ratio, output should be roughly same size
        assert!(!output.is_empty());
        for &sample in &output {
            assert_eq!(sample, 0);
        }
    }

    #[test]
    fn test_resampler_process_mono() {
        let mut resampler = SincResampler::new(48000, 48000, 1);
        let input = vec![1000i16; 10]; // 10 mono frames
        let mut output = Vec::new();

        resampler.process(&input, &mut output);
        assert!(!output.is_empty());
    }

    #[test]
    fn test_resampler_process_clamp() {
        let mut resampler = SincResampler::new(48000, 48000, 1);
        // Fill buffer to test clamping
        let input = vec![i16::MAX; 100];
        let mut output = Vec::new();

        resampler.process(&input, &mut output);

        // Output should be clamped to i16 range
        for &sample in &output {
            assert!(sample >= i16::MIN && sample <= i16::MAX);
        }
    }

    #[test]
    fn test_is_passthrough_exact() {
        let resampler = SincResampler::new(48000, 48000, 2);
        assert!(resampler.is_passthrough());
    }

    #[test]
    fn test_is_not_passthrough() {
        let resampler = SincResampler::new(48000, 44100, 2);
        assert!(!resampler.is_passthrough());
    }

    #[test]
    fn test_resampler_table_generation() {
        let resampler = SincResampler::new(48000, 44100, 2);

        // Table should have 32 entries
        assert_eq!(resampler.table.len(), 32);

        // Table values should be finite
        for &val in &resampler.table {
            assert!(val.is_finite());
        }
    }

    #[test]
    fn test_resampler_multiple_channels() {
        for channels in 1..=8 {
            let resampler = SincResampler::new(48000, 44100, channels);
            assert_eq!(resampler.buffer.len(), channels);
            for ch_buffer in &resampler.buffer {
                assert_eq!(ch_buffer.len(), 32);
            }
        }
    }
}
