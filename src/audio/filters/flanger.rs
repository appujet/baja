use super::{AudioFilter, delay_line::DelayLine, lfo::Lfo};

const MAX_DELAY_MS: f32 = 10.0;
const BUFFER_SIZE: usize = ((48000.0 * MAX_DELAY_MS) / 1000.0) as usize;

pub struct FlangerFilter {
    rate: f32,
    depth: f32,
    feedback: f32,

    lfo: Lfo,
    delay_line: DelayLine,
}

impl FlangerFilter {
    pub fn new(rate: f32, depth: f32, feedback: f32) -> Self {
        let mut filter = Self {
            rate: 0.0,
            depth: 0.0,
            feedback: 0.0,
            lfo: Lfo::new(),
            delay_line: DelayLine::new(BUFFER_SIZE),
        };
        filter.update(rate, depth, feedback);
        filter
    }

    pub fn update(&mut self, rate: f32, depth: f32, feedback: f32) {
        self.rate = rate;
        self.depth = depth.clamp(0.0, 1.0);
        self.feedback = feedback.clamp(0.0, 0.95);

        self.lfo.update(self.rate.into(), self.depth.into());
    }
}

impl AudioFilter for FlangerFilter {
    fn process(&mut self, samples: &mut [i16]) {
        if self.rate == 0.0 || self.depth == 0.0 {
            return;
        }

        let fs = 48000.0;
        let max_delay_width = self.depth * (fs * 0.005);
        let center_delay = max_delay_width;

        // Note:  processes mono chunk equivalents (i+=2), but works on both channels equally
        for sample in samples.iter_mut() {
            let lfo_value = self.lfo.get_value() as f32;
            let delay = center_delay + lfo_value * max_delay_width;

            let delayed = self.delay_line.read(delay.into());
            let input = (*sample as f32) + delayed * self.feedback;
            self.delay_line
                .write(input.clamp(i16::MIN as f32, i16::MAX as f32));

            let output = (*sample as f32) + delayed;
            *sample = output.clamp(i16::MIN as f32, i16::MAX as f32) as i16;
        }
    }

    fn is_enabled(&self) -> bool {
        self.rate > 0.0 && self.depth > 0.0
    }

    fn reset(&mut self) {
        self.delay_line.clear();
        self.lfo.set_phase(0.0);
    }
}
