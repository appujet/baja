use super::AudioFilter;
use super::delay_line::DelayLine;
use super::lfo::Lfo;

const SAMPLE_RATE: f64 = 48000.0;
const MAX_DELAY_MS: f64 = 20.0;

/// Vibrato filter — LFO-driven delay modulation.
pub struct VibratoFilter {
    lfo: Lfo,
    left_delay: DelayLine,
    right_delay: DelayLine,
}

impl VibratoFilter {
    pub fn new(frequency: f32, depth: f32) -> Self {
        let buffer_size = ((SAMPLE_RATE * MAX_DELAY_MS) / 1000.0).ceil() as usize;
        let mut lfo = Lfo::new();
        let depth = depth.clamp(0.0, 2.0);
        lfo.update(frequency as f64, depth as f64);

        Self {
            lfo,
            left_delay: DelayLine::new(buffer_size),
            right_delay: DelayLine::new(buffer_size),
        }
    }
}

impl AudioFilter for VibratoFilter {
    fn process(&mut self, samples: &mut [i16]) {
        if self.lfo.depth == 0.0 || self.lfo.frequency == 0.0 {
            self.left_delay.clear();
            self.right_delay.clear();
            return;
        }

        let max_delay_width = self.lfo.depth * SAMPLE_RATE * 0.005;
        let center_delay = max_delay_width;
        let num_frames = samples.len() / 2;

        for frame in 0..num_frames {
            let offset = frame * 2;
            let lfo_value = self.lfo.get_value();
            let delay = center_delay + lfo_value * max_delay_width;

            // Left channel
            let left_sample = samples[offset] as f32;
            self.left_delay.write(left_sample);
            let delayed_left = self.left_delay.read(delay);
            samples[offset] = (delayed_left as i32).clamp(i16::MIN as i32, i16::MAX as i32) as i16;

            // Right channel
            let right_sample = samples[offset + 1] as f32;
            self.right_delay.write(right_sample);
            let delayed_right = self.right_delay.read(delay);
            samples[offset + 1] =
                (delayed_right as i32).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        }
    }

    fn is_enabled(&self) -> bool {
        self.lfo.depth > 0.0 && self.lfo.frequency > 0.0
    }

    fn reset(&mut self) {
        self.lfo.reset();
        self.left_delay.clear();
        self.right_delay.clear();
    }
}
