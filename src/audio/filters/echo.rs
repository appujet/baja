use std::collections::VecDeque;

use super::AudioFilter;

pub struct EchoFilter {
    echo_length: f32, // in seconds (e.g. 1.0 = 1s)
    decay: f32,       // feedback multiplier (e.g. 0.5)

    // Buffer layout: interleaved L/R samples, so length is frames * 2.
    // 1 second at 48000Hz = 96000 samples.
    buffer: VecDeque<i16>,
    delay_samples: usize,
}

impl EchoFilter {
    pub fn new(echo_length: f32, decay: f32) -> Self {
        // Clamp to avoid excessive memory or invalid values
        let length = echo_length.clamp(0.001, 5.0);
        let decay = decay.clamp(0.0, 1.0);

        let frames = (48000.0 * length) as usize;
        let samples = frames * 2;

        let mut buffer = VecDeque::with_capacity(samples);
        buffer.extend(std::iter::repeat(0).take(samples));

        Self {
            echo_length: length,
            decay,
            buffer,
            delay_samples: samples,
        }
    }
}

impl AudioFilter for EchoFilter {
    fn process(&mut self, samples: &mut [i16]) {
        if self.echo_length <= 0.0 || self.decay <= 0.0 {
            return;
        }

        for sample in samples.iter_mut() {
            // Read from delay line
            let delayed = self.buffer.pop_front().unwrap_or(0);

            // Mix original with delayed
            let mixed = (*sample as f32) + (delayed as f32 * self.decay);
            // Clamp and update sample
            let out = mixed.clamp(i16::MIN as f32, i16::MAX as f32) as i16;
            *sample = out;

            // Push output back into delay line for feedback echo
            self.buffer.push_back(out);
        }
    }

    fn is_enabled(&self) -> bool {
        self.echo_length > 0.0 && self.decay > 0.0
    }

    fn reset(&mut self) {
        self.buffer.clear();
        self.buffer
            .extend(std::iter::repeat(0).take(self.delay_samples));
    }
}
