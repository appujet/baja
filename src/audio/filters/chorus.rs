use super::{delay_line::DelayLine, lfo::Lfo, AudioFilter};

const MAX_DELAY_MS: f32 = 50.0;
const BUFFER_SIZE: usize = ((48000.0 * MAX_DELAY_MS) / 1000.0) as usize;

pub struct ChorusFilter {
    rate: f32,
    depth: f32,
    delay: f32,
    mix: f32,
    feedback: f32,

    lfos: [Lfo; 4],
    delays: [DelayLine; 4],
}

impl ChorusFilter {
    pub fn new(rate: f32, depth: f32, delay: f32, mix: f32, feedback: f32) -> Self {
        let mut filter = Self {
            rate: 0.0,
            depth: 0.0,
            delay: 25.0,
            mix: 0.5,
            feedback: 0.0,
            lfos: [
                Lfo::new(),
                Lfo::new(),
                Lfo::new(),
                Lfo::new(),
            ],
            delays: [
                DelayLine::new(BUFFER_SIZE),
                DelayLine::new(BUFFER_SIZE),
                DelayLine::new(BUFFER_SIZE),
                DelayLine::new(BUFFER_SIZE),
            ],
        };

        filter.lfos[0].set_phase(0.0);
        filter.lfos[1].set_phase(std::f64::consts::PI / 2.0);
        filter.lfos[2].set_phase(std::f64::consts::PI);
        filter.lfos[3].set_phase(3.0 * std::f64::consts::PI / 2.0);

        filter.update(rate, depth, delay, mix, feedback);
        filter
    }

    pub fn update(&mut self, rate: f32, depth: f32, delay: f32, mix: f32, feedback: f32) {
        self.rate = rate;
        self.depth = depth.clamp(0.0, 1.0);
        self.delay = delay.clamp(1.0, MAX_DELAY_MS - 5.0);
        self.mix = mix.clamp(0.0, 1.0);
        self.feedback = feedback.clamp(0.0, 0.95);

        let rate2 = self.rate * 1.1;

        self.lfos[0].update(self.rate.into(), self.depth.into());
        self.lfos[1].update(self.rate.into(), self.depth.into());
        self.lfos[2].update(rate2.into(), self.depth.into());
        self.lfos[3].update(rate2.into(), self.depth.into());
    }
}

impl AudioFilter for ChorusFilter {
    fn process(&mut self, samples: &mut [i16]) {
        if self.rate == 0.0 || self.depth == 0.0 || self.mix == 0.0 {
            return;
        }

        let fs = 48000.0;
        let delay_width = self.depth * (fs * 0.004);
        let center_delay_samples = self.delay * (fs / 1000.0);
        let center_delay_samples2 = center_delay_samples * 1.2;

        for chunk in samples.chunks_exact_mut(2) {
            let left_in = chunk[0] as f32;
            let right_in = chunk[1] as f32;

            let lfo1_l = self.lfos[0].get_value() as f32;
            let lfo1_r = self.lfos[1].get_value() as f32;
            let delay1_l = center_delay_samples + lfo1_l * delay_width;
            let delay1_r = center_delay_samples + lfo1_r * delay_width;
            let delayed1_l = self.delays[0].read(delay1_l);
            let delayed1_r = self.delays[1].read(delay1_r);

            let lfo2_l = self.lfos[2].get_value() as f32;
            let lfo2_r = self.lfos[3].get_value() as f32;
            let delay2_l = center_delay_samples2 + lfo2_l * delay_width;
            let delay2_r = center_delay_samples2 + lfo2_r * delay_width;
            let delayed2_l = self.delays[2].read(delay2_l);
            let delayed2_r = self.delays[3].read(delay2_r);

            let wet_left = (delayed1_l + delayed2_l) * 0.5;
            let wet_right = (delayed1_r + delayed2_r) * 0.5;

            let final_left = left_in * (1.0 - self.mix) + wet_left * self.mix;
            let final_right = right_in * (1.0 - self.mix) + wet_right * self.mix;

            self.delays[0].write((left_in + delayed1_l * self.feedback).clamp(i16::MIN as f32, i16::MAX as f32));
            self.delays[1].write((right_in + delayed1_r * self.feedback).clamp(i16::MIN as f32, i16::MAX as f32));
            self.delays[2].write((left_in + delayed2_l * self.feedback).clamp(i16::MIN as f32, i16::MAX as f32));
            self.delays[3].write((right_in + delayed2_r * self.feedback).clamp(i16::MIN as f32, i16::MAX as f32));

            chunk[0] = final_left.clamp(i16::MIN as f32, i16::MAX as f32) as i16;
            chunk[1] = final_right.clamp(i16::MIN as f32, i16::MAX as f32) as i16;
        }
    }

    fn is_enabled(&self) -> bool {
        self.rate > 0.0 && self.depth > 0.0 && self.mix > 0.0
    }

    fn reset(&mut self) {
        for delay in &mut self.delays {
            delay.clear();
        }
        self.lfos[0].set_phase(0.0);
        self.lfos[1].set_phase(std::f64::consts::PI / 2.0);
        self.lfos[2].set_phase(std::f64::consts::PI);
        self.lfos[3].set_phase(3.0 * std::f64::consts::PI / 2.0);
    }
}
