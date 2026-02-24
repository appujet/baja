use super::{AudioFilter, delay_line::DelayLine, lfo::Lfo};

const MAX_DELAY_MS: f32 = 30.0;
const BUFFER_SIZE: usize = ((48000.0 * MAX_DELAY_MS) / 1000.0) as usize;

pub struct SpatialFilter {
    depth: f32,
    rate: f32,

    left_delay: DelayLine,
    right_delay: DelayLine,
    lfo: Lfo,
}

impl SpatialFilter {
    pub fn new(rate: f32, depth: f32) -> Self {
        let mut filter = Self {
            depth: 0.0,
            rate: 0.0,
            left_delay: DelayLine::new(BUFFER_SIZE),
            right_delay: DelayLine::new(BUFFER_SIZE),
            lfo: Lfo::new(),
        };
        filter.update(rate, depth);
        filter
    }

    pub fn update(&mut self, rate: f32, depth: f32) {
        self.rate = rate;
        self.depth = depth.clamp(0.0, 1.0);
        self.lfo.update(self.rate.into(), 1.0);
    }
}

impl AudioFilter for SpatialFilter {
    fn process(&mut self, samples: &mut [i16]) {
        if self.depth == 0.0 {
            return;
        }

        let fs = 48000.0;
        let wet = self.depth * 0.5;
        let dry = 1.0 - wet;
        let feedback = -0.3;

        for chunk in samples.chunks_exact_mut(2) {
            let left_in = chunk[0] as f32;
            let right_in = chunk[1] as f32;

            let lfo_value = self.lfo.get_value() as f32;

            let delay_time_l = (5.0 + lfo_value * 2.0) * (fs / 1000.0);
            let delay_time_r = (5.0 - lfo_value * 2.0) * (fs / 1000.0);

            let delayed_left = self.left_delay.read(delay_time_l.into());
            let delayed_right = self.right_delay.read(delay_time_r.into());

            self.left_delay
                .write((left_in + delayed_left * feedback).clamp(i16::MIN as f32, i16::MAX as f32));
            self.right_delay.write(
                (right_in + delayed_right * feedback).clamp(i16::MIN as f32, i16::MAX as f32),
            );

            let new_left = left_in * dry + delayed_right * wet;
            let new_right = right_in * dry + delayed_left * wet;

            chunk[0] = new_left.clamp(i16::MIN as f32, i16::MAX as f32) as i16;
            chunk[1] = new_right.clamp(i16::MIN as f32, i16::MAX as f32) as i16;
        }
    }

    fn is_enabled(&self) -> bool {
        self.depth > 0.0
    }

    fn reset(&mut self) {
        self.left_delay.clear();
        self.right_delay.clear();
    }
}
