use super::{delay_line::DelayLine, AudioFilter};

const COMB_DELAYS: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASS_DELAYS: [usize; 4] = [556, 441, 341, 225];
const STEREO_SPREAD: usize = 23;
const SCALE_WET: f32 = 3.0;
const SCALE_DRY: f32 = 2.0;
const SCALE_DAMP: f32 = 0.4;
const SCALE_ROOM: f32 = 0.28;
const OFFSET_ROOM: f32 = 0.7;

struct CombFilter {
    buffer: DelayLine,
    filter_store: f32,
    damp1: f32,
    damp2: f32,
    feedback: f32,
}

impl CombFilter {
    fn new(size: usize) -> Self {
        Self {
            buffer: DelayLine::new(size),
            filter_store: 0.0,
            damp1: 0.0,
            damp2: 0.0,
            feedback: 0.0,
        }
    }

    fn set_damp(&mut self, val: f32) {
        self.damp1 = val;
        self.damp2 = 1.0 - val;
    }

    fn set_feedback(&mut self, val: f32) {
        self.feedback = val;
    }

    fn process(&mut self, input: f32) -> f32 {
        let output = self.buffer.read(0.0);
        self.filter_store = output * self.damp2 + self.filter_store * self.damp1;
        
        let write_val = input + self.filter_store * self.feedback;
        self.buffer.write(write_val.clamp(i16::MIN as f32, i16::MAX as f32));
        output
    }

    fn clear(&mut self) {
        self.buffer.clear();
        self.filter_store = 0.0;
    }
}

pub struct ReverbFilter {
    comb_l: Vec<CombFilter>,
    comb_r: Vec<CombFilter>,
    allpass_l: Vec<DelayLine>,
    allpass_r: Vec<DelayLine>,
    allpass_coeff: f32,
    allpass_state_l: Vec<f32>,
    allpass_state_r: Vec<f32>,

    wet: f32,
    dry: f32,
    room_size: f32,
    damping: f32,
    width: f32,
}

impl ReverbFilter {
    pub fn new(mix: f32, room_size: f32, damping: f32, width: f32) -> Self {
        let fs = 48000.0;

        let comb_l = COMB_DELAYS.iter().map(|&d| CombFilter::new((d as f64 * fs / 44100.0) as usize)).collect();
        let comb_r = COMB_DELAYS.iter().map(|&d| CombFilter::new(((d + STEREO_SPREAD) as f64 * fs / 44100.0) as usize)).collect();
        let allpass_l = ALLPASS_DELAYS.iter().map(|&d| DelayLine::new((d as f64 * fs / 44100.0) as usize)).collect();
        let allpass_r = ALLPASS_DELAYS.iter().map(|&d| DelayLine::new(((d + STEREO_SPREAD) as f64 * fs / 44100.0) as usize)).collect();

        let mut filter = Self {
            comb_l,
            comb_r,
            allpass_l,
            allpass_r,
            allpass_coeff: 0.5,
            allpass_state_l: vec![0.0; ALLPASS_DELAYS.len()],
            allpass_state_r: vec![0.0; ALLPASS_DELAYS.len()],

            wet: 0.0,
            dry: 1.0,
            room_size: 0.5,
            damping: 0.5,
            width: 1.0,
        };

        filter.update(mix, room_size, damping, width);
        filter
    }

    pub fn update(&mut self, mix: f32, room_size: f32, damping: f32, width: f32) {
        let mix = mix.clamp(0.0, 1.0);
        self.wet = mix * SCALE_WET;
        self.dry = (1.0 - mix) * SCALE_DRY;

        self.room_size = room_size.clamp(0.0, 1.0);
        let room_scaled = self.room_size * SCALE_ROOM + OFFSET_ROOM;

        self.damping = damping.clamp(0.0, 1.0);
        let damp_scaled = self.damping * SCALE_DAMP;

        self.width = width.clamp(0.0, 1.0);

        for comb in self.comb_l.iter_mut() {
            comb.set_feedback(room_scaled);
            comb.set_damp(damp_scaled);
        }
        for comb in self.comb_r.iter_mut() {
            comb.set_feedback(room_scaled);
            comb.set_damp(damp_scaled);
        }
    }

    fn process_allpass(input: f32, delay_line: &mut DelayLine, state_y1: &mut f32, coeff: f32) -> f32 {
        let delayed = delay_line.read(0.0);
        let output = -input + delayed + coeff * (input - *state_y1);
        
        delay_line.write(input.clamp(i16::MIN as f32, i16::MAX as f32));
        *state_y1 = output;
        
        output
    }
}

impl AudioFilter for ReverbFilter {
    fn process(&mut self, samples: &mut [i16]) {
        if self.wet == 0.0 {
            return;
        }

        for chunk in samples.chunks_exact_mut(2) {
            let left_input = chunk[0] as f32;
            let right_input = chunk[1] as f32;
            let mono_input = (left_input + right_input) * 0.5;

            let mut left_out = 0.0;
            let mut right_out = 0.0;

            for j in 0..self.comb_l.len() {
                left_out += self.comb_l[j].process(mono_input);
                right_out += self.comb_r[j].process(mono_input);
            }

            for j in 0..self.allpass_l.len() {
                left_out = Self::process_allpass(left_out, &mut self.allpass_l[j], &mut self.allpass_state_l[j], self.allpass_coeff);
                right_out = Self::process_allpass(right_out, &mut self.allpass_r[j], &mut self.allpass_state_r[j], self.allpass_coeff);
            }

            let wet1 = self.wet * (self.width * 0.5 + 0.5);
            let wet2 = self.wet * ((1.0 - self.width) * 0.5);

            let final_left = left_input * self.dry + left_out * wet1 + right_out * wet2;
            let final_right = right_input * self.dry + right_out * wet1 + left_out * wet2;

            chunk[0] = final_left.clamp(i16::MIN as f32, i16::MAX as f32) as i16;
            chunk[1] = final_right.clamp(i16::MIN as f32, i16::MAX as f32) as i16;
        }
    }

    fn is_enabled(&self) -> bool {
        self.wet > 0.0
    }

    fn reset(&mut self) {
        for comb in self.comb_l.iter_mut() { comb.clear(); }
        for comb in self.comb_r.iter_mut() { comb.clear(); }
        for pass in self.allpass_l.iter_mut() { pass.clear(); }
        for pass in self.allpass_r.iter_mut() { pass.clear(); }
        for state in self.allpass_state_l.iter_mut() { *state = 0.0; }
        for state in self.allpass_state_r.iter_mut() { *state = 0.0; }
    }
}
