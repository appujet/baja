use super::{AudioFilter, lfo::Lfo};

const MAX_STAGES: usize = 12;

struct Allpass {
    a1: f32,
    z1: f32,
}

impl Allpass {
    fn new() -> Self {
        Self { a1: 0.0, z1: 0.0 }
    }

    fn set_coefficient(&mut self, coef: f32) {
        self.a1 = coef;
    }

    fn process(&mut self, input: f32) -> f32 {
        let output = input * -self.a1 + self.z1;
        self.z1 = output * self.a1 + input;
        output
    }

    fn reset(&mut self) {
        self.z1 = 0.0;
    }
}

pub struct PhaserFilter {
    stages: usize,
    rate: f32,
    depth: f32,
    feedback: f32,
    mix: f32,
    min_frequency: f32,
    max_frequency: f32,

    left_lfo: Lfo,
    right_lfo: Lfo,

    left_filters: Vec<Allpass>,
    right_filters: Vec<Allpass>,

    last_left_feedback: f32,
    last_right_feedback: f32,
}

impl PhaserFilter {
    pub fn new(
        stages: i32,
        rate: f32,
        depth: f32,
        feedback: f32,
        mix: f32,
        min_frequency: f32,
        max_frequency: f32,
    ) -> Self {
        let mut left_filters = Vec::with_capacity(MAX_STAGES);
        let mut right_filters = Vec::with_capacity(MAX_STAGES);
        for _ in 0..MAX_STAGES {
            left_filters.push(Allpass::new());
            right_filters.push(Allpass::new());
        }

        let mut right_lfo = Lfo::new();
        right_lfo.set_phase(std::f64::consts::PI / 2.0);

        let mut filter = Self {
            stages: 4,
            rate: 0.0,
            depth: 1.0,
            feedback: 0.0,
            mix: 0.5,
            min_frequency: 100.0,
            max_frequency: 2500.0,

            left_lfo: Lfo::new(),
            right_lfo,

            left_filters,
            right_filters,

            last_left_feedback: 0.0,
            last_right_feedback: 0.0,
        };

        filter.update(
            stages,
            rate,
            depth,
            feedback,
            mix,
            min_frequency,
            max_frequency,
        );
        filter
    }

    pub fn update(
        &mut self,
        stages: i32,
        rate: f32,
        depth: f32,
        feedback: f32,
        mix: f32,
        min_frequency: f32,
        max_frequency: f32,
    ) {
        self.stages = (stages as usize).clamp(2, MAX_STAGES);
        self.rate = rate;
        self.depth = depth.clamp(0.0, 1.0);
        self.feedback = feedback.clamp(0.0, 0.9);
        self.mix = mix.clamp(0.0, 1.0);
        self.min_frequency = min_frequency;
        self.max_frequency = max_frequency;

        self.left_lfo.update(self.rate.into(), self.depth.into());
        self.right_lfo.update(self.rate.into(), self.depth.into());
    }
}

impl AudioFilter for PhaserFilter {
    fn process(&mut self, samples: &mut [i16]) {
        if self.rate == 0.0 || self.depth == 0.0 || self.mix == 0.0 {
            return;
        }

        let fs = 48000.0;
        let sweep_range = self.max_frequency - self.min_frequency;

        for chunk in samples.chunks_exact_mut(2) {
            let left_sample = chunk[0] as f32;
            let right_sample = chunk[1] as f32;

            let left_lfo_val = (self.left_lfo.get_value() as f32 + 1.0) / 2.0;
            let right_lfo_val = (self.right_lfo.get_value() as f32 + 1.0) / 2.0;

            let current_left_freq = self.min_frequency + sweep_range * left_lfo_val;
            let current_right_freq = self.min_frequency + sweep_range * right_lfo_val;

            let tan_left = (std::f32::consts::PI * current_left_freq / fs).tan();
            let a_left = (1.0 - tan_left) / (1.0 + tan_left);

            let tan_right = (std::f32::consts::PI * current_right_freq / fs).tan();
            let a_right = (1.0 - tan_right) / (1.0 + tan_right);

            let mut wet_left = left_sample + self.last_left_feedback * self.feedback;
            for j in 0..self.stages {
                self.left_filters[j].set_coefficient(a_left);
                wet_left = self.left_filters[j].process(wet_left);
            }
            self.last_left_feedback = wet_left;
            let final_left = left_sample * (1.0 - self.mix) + wet_left * self.mix;

            let mut wet_right = right_sample + self.last_right_feedback * self.feedback;
            for j in 0..self.stages {
                self.right_filters[j].set_coefficient(a_right);
                wet_right = self.right_filters[j].process(wet_right);
            }
            self.last_right_feedback = wet_right;
            let final_right = right_sample * (1.0 - self.mix) + wet_right * self.mix;

            chunk[0] = final_left.clamp(i16::MIN as f32, i16::MAX as f32) as i16;
            chunk[1] = final_right.clamp(i16::MIN as f32, i16::MAX as f32) as i16;
        }
    }

    fn is_enabled(&self) -> bool {
        self.rate > 0.0 && self.depth > 0.0 && self.mix > 0.0
    }

    fn reset(&mut self) {
        for filter in &mut self.left_filters {
            filter.reset();
        }
        for filter in &mut self.right_filters {
            filter.reset();
        }
        self.last_left_feedback = 0.0;
        self.last_right_feedback = 0.0;
        self.left_lfo.set_phase(0.0);
        self.right_lfo.set_phase(std::f64::consts::PI / 2.0);
    }
}
