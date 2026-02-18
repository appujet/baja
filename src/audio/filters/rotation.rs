use super::AudioFilter;
use super::lfo::Lfo;

/// Rotation (8D audio) filter — LFO-driven stereo panning.
pub struct RotationFilter {
    lfo: Lfo,
}

impl RotationFilter {
    pub fn new(rotation_hz: f64) -> Self {
        let mut lfo = Lfo::new();
        lfo.update(rotation_hz, 1.0);
        Self { lfo }
    }
}

impl AudioFilter for RotationFilter {
    fn process(&mut self, samples: &mut [i16]) {
        if self.lfo.frequency == 0.0 {
            return;
        }

        let num_frames = samples.len() / 2;
        for frame in 0..num_frames {
            let offset = frame * 2;
            let lfo_value = self.lfo.get_value();

            let left_factor = (1.0 - lfo_value) / 2.0;
            let right_factor = (1.0 + lfo_value) / 2.0;

            let left = samples[offset] as f64;
            let right = samples[offset + 1] as f64;

            let new_left = (left * left_factor) as i32;
            let new_right = (right * right_factor) as i32;

            samples[offset] = new_left.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
            samples[offset + 1] = new_right.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        }
    }

    fn is_enabled(&self) -> bool {
        self.lfo.frequency != 0.0
    }

    fn reset(&mut self) {
        self.lfo.reset();
    }
}
