use std::collections::VecDeque;

use crate::audio::buffer::PooledBuffer;

pub struct SincResampler {
    ratio: f32,
    index: f32,
    channels: usize,
    taps: usize,
    buffer: Vec<VecDeque<f32>>,
}

impl SincResampler {
    pub fn new(source_rate: u32, target_rate: u32, channels: usize) -> Self {
        let taps = 32;
        Self {
            ratio: source_rate as f32 / target_rate as f32,
            index: 0.0,
            channels,
            taps,
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

    fn blackman(n: f32, m: f32) -> f32 {
        let a0 = 0.42;
        let a1 = 0.5;
        let a2 = 0.08;
        let pi_n_m = 2.0 * std::f32::consts::PI * n / m;
        a0 - a1 * pi_n_m.cos() + a2 * (2.0 * pi_n_m).cos()
    }

    pub fn process(&mut self, input: &[i16], output: &mut PooledBuffer) {
        let num_frames = input.len() / self.channels;
        let half_taps = (self.taps / 2) as f32;

        for frame in 0..num_frames {
            for ch in 0..self.channels {
                self.buffer[ch].pop_front();
                self.buffer[ch].push_back(input[frame * self.channels + ch] as f32);
            }

            while self.index < 1.0 {
                for ch in 0..self.channels {
                    let mut sum = 0.0;

                    for i in 0..self.taps {
                        let offset = (i as f32 - half_taps) - self.index;
                        let window = Self::blackman(i as f32, self.taps as f32 - 1.0);
                        sum += self.buffer[ch][i] * Self::sinc(offset) * window;
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
