//! `resample/linear.rs` — fast linear-interpolation resampler.
//!
//! Identical algorithm to the original `pipeline/resampler.rs`, now exposed
//! as a named type so callers can choose quality vs. speed.

pub struct LinearResampler {
    /// Source / target ratio (< 1.0 upsamples, > 1.0 downsamples).
    ratio: f32,
    /// Fractional read head within the current input block.
    index: f32,
    /// Last sample of the previous block (per channel) for cross-block interpolation.
    last_samples: Vec<i16>,
    channels: usize,
}

impl LinearResampler {
    pub fn new(source_rate: u32, target_rate: u32, channels: usize) -> Self {
        Self {
            ratio: source_rate as f32 / target_rate as f32,
            index: 0.0,
            last_samples: vec![0; channels],
            channels,
        }
    }

    /// Resample `input` and **append** resampled samples into `output`.
    pub fn process(&mut self, input: &[i16], output: &mut Vec<i16>) {
        let num_frames = input.len() / self.channels;

        while self.index < num_frames as f32 {
            let idx = self.index as usize;
            let fract = self.index.fract();

            for c in 0..self.channels {
                let s1 = if idx == 0 {
                    self.last_samples[c] as f32
                } else {
                    input[(idx - 1) * self.channels + c] as f32
                };

                let s2 = if idx < num_frames {
                    input[idx * self.channels + c] as f32
                } else {
                    input[(num_frames - 1) * self.channels + c] as f32
                };

                output.push((s1 * (1.0 - fract) + s2 * fract) as i16);
            }

            self.index += self.ratio;
        }

        self.index -= num_frames as f32;

        if num_frames > 0 {
            for c in 0..self.channels {
                self.last_samples[c] = input[(num_frames - 1) * self.channels + c];
            }
        }
    }

    pub fn reset(&mut self) {
        self.index = 0.0;
        self.last_samples.fill(0);
    }

    pub fn is_passthrough(&self) -> bool {
        (self.ratio - 1.0).abs() < f32::EPSILON
    }
}
