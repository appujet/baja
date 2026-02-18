use super::AudioFilter;
use super::biquad::{BiquadCoeffs, BiquadState};

const SAMPLE_RATE: f64 = 48000.0;
const SCALE_16: f64 = 32768.0;
const INV_16: f64 = 1.0 / SCALE_16;
const MAX_OUTPUT_GAIN: f64 = 0.98;

/// Karaoke filter — vocal cancellation via LP/HP biquad filters + mid-channel subtraction.
pub struct KaraokeFilter {
    level: f32,
    mono_level: f32,
    filter_band: f32,
    filter_width: f32,
    lp_coeffs: BiquadCoeffs,
    hp_coeffs: BiquadCoeffs,
    // Per-channel LP/HP states: [left, right]
    lp_states: [BiquadState; 2],
    hp_states: [BiquadState; 2],
    prev_gain: f64,
}

impl KaraokeFilter {
    pub fn new(level: f32, mono_level: f32, filter_band: f32, filter_width: f32) -> Self {
        let level = level.clamp(0.0, 1.0);
        let mono_level = mono_level.clamp(0.0, 1.0);

        let (lp_coeffs, hp_coeffs) =
            Self::compute_coefficients(filter_band as f64, filter_width as f64);

        Self {
            level,
            mono_level,
            filter_band,
            filter_width,
            lp_coeffs,
            hp_coeffs,
            lp_states: [BiquadState::default(), BiquadState::default()],
            hp_states: [BiquadState::default(), BiquadState::default()],
            prev_gain: MAX_OUTPUT_GAIN,
        }
    }

    fn compute_coefficients(band: f64, width: f64) -> (BiquadCoeffs, BiquadCoeffs) {
        if band <= 0.0 || width <= 0.0 {
            // Passthrough coefficients
            let passthrough = BiquadCoeffs {
                b0: 1.0,
                b1: 0.0,
                b2: 0.0,
                a1: 0.0,
                a2: 0.0,
            };
            return (passthrough.clone(), passthrough);
        }

        let fc = band.max(1.0).min(SAMPLE_RATE * 0.49);
        let w = width.max(1e-6);
        let q = (fc / w).max(1e-4);

        let lp = BiquadCoeffs::lowpass(fc, q, SAMPLE_RATE);
        let hp = BiquadCoeffs::highpass(fc, q, SAMPLE_RATE);
        (lp, hp)
    }
}

impl AudioFilter for KaraokeFilter {
    fn process(&mut self, samples: &mut [i16]) {
        if self.level <= 0.0 && self.mono_level <= 0.0 {
            return;
        }

        let num_frames = samples.len() / 2;
        if num_frames == 0 {
            return;
        }

        let do_filter =
            self.level > 0.0 && self.filter_band > 0.0 && self.filter_width > 0.0;

        // First pass: compute filtered output + energy
        let mut out_left_buf = vec![0.0f64; num_frames];
        let mut out_right_buf = vec![0.0f64; num_frames];
        let mut original_energy = 0.0f64;
        let mut processed_energy = 0.0f64;

        for frame in 0..num_frames {
            let offset = frame * 2;
            let mut left = samples[offset] as f64 * INV_16;
            let mut right = samples[offset + 1] as f64 * INV_16;

            original_energy += left * left + right * right;

            // Mono level: subtract mid from both channels
            if self.mono_level > 0.0 {
                let mid = (left + right) * 0.5;
                let sub = mid * self.mono_level as f64;
                left -= sub;
                right -= sub;
            }

            if do_filter {
                let low_left = self.lp_states[0].process(left, &self.lp_coeffs);
                let low_right = self.lp_states[1].process(right, &self.lp_coeffs);
                let high_left = self.hp_states[0].process(left, &self.hp_coeffs);
                let high_right = self.hp_states[1].process(right, &self.hp_coeffs);

                let cancelled = high_left - high_right;
                left = low_left + cancelled * self.level as f64;
                right = low_right + cancelled * self.level as f64;
            }

            out_left_buf[frame] = left;
            out_right_buf[frame] = right;
            processed_energy += left * left + right * right;
        }

        // Calculate normalization gain
        let denom = (num_frames * 2) as f64;
        original_energy /= denom;
        processed_energy /= denom;

        let gain = if processed_energy > 1e-15 {
            let g = (original_energy.max(1e-12) / processed_energy).sqrt();
            g.min(MAX_OUTPUT_GAIN)
        } else {
            MAX_OUTPUT_GAIN
        };

        // Smooth gain transition
        let smooth = if gain > self.prev_gain { 0.06 } else { 0.3 };
        let target = self.prev_gain + (gain - self.prev_gain) * smooth;
        let step = (target - self.prev_gain) / num_frames as f64;
        let mut current = self.prev_gain;

        // Second pass: apply gain and write output
        for frame in 0..num_frames {
            let offset = frame * 2;
            current += step;

            let mut out_l = out_left_buf[frame] * current;
            let mut out_r = out_right_buf[frame] * current;

            // Peak limiter
            let peak = out_l.abs().max(out_r.abs());
            if peak > 0.9999 {
                let s = 0.9999 / peak;
                out_l *= s;
                out_r *= s;
            }

            samples[offset] =
                ((out_l * SCALE_16).round() as i32).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
            samples[offset + 1] =
                ((out_r * SCALE_16).round() as i32).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        }

        self.prev_gain = target;
    }

    fn is_enabled(&self) -> bool {
        self.level > 0.0 || self.mono_level > 0.0
    }

    fn reset(&mut self) {
        for s in self.lp_states.iter_mut() {
            s.reset();
        }
        for s in self.hp_states.iter_mut() {
            s.reset();
        }
        self.prev_gain = MAX_OUTPUT_GAIN;
    }
}
