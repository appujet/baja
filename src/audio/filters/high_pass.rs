use super::{biquad::{BiquadCoeffs, BiquadState}, AudioFilter};

pub struct HighPassFilter {
    cutoff_frequency: i32,
    boost_factor: f32,
    
    left_state: BiquadState,
    right_state: BiquadState,
    coeffs: Option<BiquadCoeffs>,
}

impl HighPassFilter {
    pub fn new(cutoff_frequency: i32, boost_factor: f32) -> Self {
        let mut filter = Self {
            cutoff_frequency,
            boost_factor,
            left_state: BiquadState::default(),
            right_state: BiquadState::default(),
            coeffs: None,
        };
        filter.update_coefficients();
        filter
    }

    fn update_coefficients(&mut self) {
        if self.cutoff_frequency <= 0 {
            return;
        }

        let fs = 48000.0;
        let fc = self.cutoff_frequency as f64;
        let q = 0.7071067811865475; // 1 / sqrt(2)
        
        // standard biquad highpass
        let w0 = 2.0 * std::f64::consts::PI * (fc / fs);
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let alpha = sin_w0 / (2.0 * q);

        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha;

        let b0 = (1.0 + cos_w0) / 2.0;
        let b1 = -(1.0 + cos_w0);
        let b2 = (1.0 + cos_w0) / 2.0;

        self.coeffs = Some(BiquadCoeffs {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
        });
    }
}

impl AudioFilter for HighPassFilter {
    fn process(&mut self, samples: &mut [i16]) {
        if self.cutoff_frequency <= 0 {
            return;
        }

        let coeffs = match &self.coeffs {
            Some(c) => c,
            None => return,
        };

        let boost_factor_f32 = self.boost_factor;

        // Process interleaved stereo
        for chunk in samples.chunks_exact_mut(2) {
            let left_in = chunk[0] as f32;
            let right_in = chunk[1] as f32;

            let left_out = self.left_state.process(left_in as f64, coeffs) as f32 * boost_factor_f32;
            let right_out = self.right_state.process(right_in as f64, coeffs) as f32 * boost_factor_f32;

            chunk[0] = left_out.clamp(i16::MIN as f32, i16::MAX as f32) as i16;
            chunk[1] = right_out.clamp(i16::MIN as f32, i16::MAX as f32) as i16;
        }
    }

    fn is_enabled(&self) -> bool {
        self.cutoff_frequency > 0
    }

    fn reset(&mut self) {
        self.left_state.reset();
        self.right_state.reset();
    }
}
