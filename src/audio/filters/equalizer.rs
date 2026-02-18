use super::AudioFilter;

const BAND_COUNT: usize = 15;
const DEFAULT_MAKEUP_GAIN: f32 = 4.0;

struct Coefficients {
    beta: f32,
    alpha: f32,
    gamma: f32,
}

// Hardcoded coefficients from LavaPlayer for 48kHz
// Order: beta, alpha, gamma
const COEFFICIENTS_48000: [Coefficients; BAND_COUNT] = [
    Coefficients { beta: 9.9847546664e-01, alpha: 7.6226668143e-04, gamma: 1.9984647656e+00 },
    Coefficients { beta: 9.9756184654e-01, alpha: 1.2190767289e-03, gamma: 1.9975344645e+00 },
    Coefficients { beta: 9.9616261379e-01, alpha: 1.9186931041e-03, gamma: 1.9960947369e+00 },
    Coefficients { beta: 9.9391578543e-01, alpha: 3.0421072865e-03, gamma: 1.9937449618e+00 },
    Coefficients { beta: 9.9028307215e-01, alpha: 4.8584639242e-03, gamma: 1.9898465702e+00 },
    Coefficients { beta: 9.8485897264e-01, alpha: 7.5705136795e-03, gamma: 1.9837962543e+00 },
    Coefficients { beta: 9.7588512657e-01, alpha: 1.2057436715e-02, gamma: 1.9731772447e+00 },
    Coefficients { beta: 9.6228521814e-01, alpha: 1.8857390928e-02, gamma: 1.9556164694e+00 },
    Coefficients { beta: 9.4080933132e-01, alpha: 2.9595334338e-02, gamma: 1.9242054384e+00 },
    Coefficients { beta: 9.0702059196e-01, alpha: 4.6489704022e-02, gamma: 1.8653476166e+00 },
    Coefficients { beta: 8.5868004289e-01, alpha: 7.0659978553e-02, gamma: 1.7600401337e+00 },
    Coefficients { beta: 7.8409610788e-01, alpha: 1.0795194606e-01, gamma: 1.5450725522e+00 },
    Coefficients { beta: 6.8332861002e-01, alpha: 1.5833569499e-01, gamma: 1.1426447155e+00 },
    Coefficients { beta: 5.5267518228e-01, alpha: 2.2366240886e-01, gamma: 4.0186190803e-01 },
    Coefficients { beta: 4.1811888447e-01, alpha: 2.9094055777e-01, gamma: -7.0905944223e-01 },
];

#[derive(Clone, Default)]
struct EqBandState {
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl EqBandState {
    fn process(&mut self, sample: f32, coeffs: &Coefficients) -> f32 {
        // LavaPlayer logic:
        // result = alpha * (sample - history[x-2]) + gamma * history[y-1] - beta * history[y-2]
        // history[x-2] corresponds to x2
        let result = coeffs.alpha * (sample - self.x2)
            + coeffs.gamma * self.y1
            - coeffs.beta * self.y2;
        
        // Update history
        self.x2 = self.x1;
        self.x1 = sample;
        self.y2 = self.y1;
        
        // Check for NaN/Inf
        if !result.is_finite() {
            self.y1 = 0.0;
            return 0.0;
        }
        
        self.y1 = result;
        result
    }
    
    fn reset(&mut self) {
        self.x1 = 0.0;
        self.x2 = 0.0;
        self.y1 = 0.0;
        self.y2 = 0.0;
    }
}

pub struct EqualizerFilter {
    gains: [f32; BAND_COUNT],
    // Per-channel state (left, right) for each band
    states: [[EqBandState; 2]; BAND_COUNT],
    makeup_gain: f32,
}

impl EqualizerFilter {
    pub fn new(bands: &[(u8, f32)]) -> Self {
        let mut gains = [0.0f32; BAND_COUNT];
        for &(band, gain) in bands {
            if (band as usize) < BAND_COUNT {
                gains[band as usize] = gain.clamp(-0.25, 1.0);
            }
        }

        let states: [[EqBandState; 2]; BAND_COUNT] =
            std::array::from_fn(|_| [EqBandState::default(), EqBandState::default()]);

        // Calculate makeup gain
        // LavaPlayer: 1.0 + (sum - 1.0) * 0.5
        let positive_sum: f32 = gains.iter().filter(|g| **g > 0.0).sum();
        let makeup_gain = if positive_sum > 1.0 {
            DEFAULT_MAKEUP_GAIN / (1.0 + (positive_sum - 1.0) * 0.5)
        } else {
            DEFAULT_MAKEUP_GAIN
        };

        Self {
            gains,
            states,
            makeup_gain,
        }
    }
}

impl AudioFilter for EqualizerFilter {
    fn process(&mut self, samples: &mut [i16]) {
        let num_frames = samples.len() / 2;

        for frame in 0..num_frames {
            let offset = frame * 2;

            let left_f = samples[offset] as f32 / 32768.0;
            let right_f = samples[offset + 1] as f32 / 32768.0;

            // Start with dry signal scaled down by 0.25
            let mut result_left = left_f * 0.25;
            let mut result_right = right_f * 0.25;

            // Sum parallel band contributions
            for b in 0..BAND_COUNT {
                let gain = self.gains[b];
                // Optimization: skip bands with effectively zero gain
                if gain.abs() < f32::EPSILON {
                    // Update state even if gain is zero to avoid clicks on gain change
                    self.states[b][0].process(left_f, &COEFFICIENTS_48000[b]);
                    self.states[b][1].process(right_f, &COEFFICIENTS_48000[b]);
                    continue;
                }

                let band_left = self.states[b][0].process(left_f, &COEFFICIENTS_48000[b]);
                let band_right = self.states[b][1].process(right_f, &COEFFICIENTS_48000[b]);

                result_left += band_left * gain;
                result_right += band_right * gain;
            }

            // Apply makeup gain and soft-clip via tanh
            let out_left = (result_left * self.makeup_gain).tanh();
            let out_right = (result_right * self.makeup_gain).tanh();

            samples[offset] = (out_left * 32767.0).round() as i16;
            samples[offset + 1] = (out_right * 32767.0).round() as i16;
        }
    }

    fn is_enabled(&self) -> bool {
        // Enabled if any band has non-zero gain
        self.gains.iter().any(|g| g.abs() > f32::EPSILON)
    }

    fn reset(&mut self) {
        for band_states in self.states.iter_mut() {
            for state in band_states.iter_mut() {
                state.reset();
            }
        }
    }
}
