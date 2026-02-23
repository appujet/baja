use super::AudioFilter;

pub struct NormalizationFilter {
    max_amplitude: f32,
    adaptive: bool,
    
    // For adaptive mode, envelope tracker
    envelope: f32,
    attack_coef: f32,
    release_coef: f32,
}

impl NormalizationFilter {
    pub fn new(max_amplitude: f32, adaptive: bool) -> Self {
        let max_amplitude = max_amplitude.max(0.01);
        
        // standard attack/release for a limiter
        // typically attack is very fast, release is slower
        let attack_ms = 1.0;
        let release_ms = 100.0;
        let fs = 48000.0;
        
        let attack_coef = (-1.0 / ((attack_ms / 1000.0) * fs) as f32).exp();
        let release_coef = (-1.0 / ((release_ms / 1000.0) * fs) as f32).exp();

        Self {
            max_amplitude,
            adaptive,
            envelope: 0.0,
            attack_coef,
            release_coef,
        }
    }
}

impl AudioFilter for NormalizationFilter {
    fn process(&mut self, samples: &mut [i16]) {
        if self.max_amplitude <= 0.0 {
            return;
        }

        if !self.adaptive {
            // Static normalization / hard limiting
            // LavaDSPX just does: `out = in * (targetMax / currentMax)` block-wise
            // But doing it sample-by-sample or hard-clipping is often safer in a streaming context
            for sample in samples.iter_mut() {
                let v = (*sample as f32) / 32768.0;
                let scaled = v.clamp(-self.max_amplitude, self.max_amplitude);
                *sample = (scaled * 32768.0) as i16;
            }
        } else {
            // Adaptive normalization (limiter approach)
            for chunk in samples.chunks_exact_mut(2) {
                let left_in = chunk[0] as f32 / 32768.0;
                let right_in = chunk[1] as f32 / 32768.0;

                let abs_peak = left_in.abs().max(right_in.abs());

                if abs_peak > self.envelope {
                    self.envelope = self.attack_coef * (self.envelope - abs_peak) + abs_peak;
                } else {
                    self.envelope = self.release_coef * (self.envelope - abs_peak) + abs_peak;
                }

                // Protect against div-by-zero
                let envelope_safe = self.envelope.max(0.001);
                
                // If the signal peak exceeds our target max_amplitude, reduce gain
                let gain = if envelope_safe > self.max_amplitude {
                    self.max_amplitude / envelope_safe
                } else {
                    1.0
                };

                chunk[0] = (left_in * gain * 32768.0).clamp(i16::MIN as f32, i16::MAX as f32) as i16;
                chunk[1] = (right_in * gain * 32768.0).clamp(i16::MIN as f32, i16::MAX as f32) as i16;
            }
        }
    }

    fn is_enabled(&self) -> bool {
        self.max_amplitude > 0.0
    }

    fn reset(&mut self) {
        self.envelope = 0.0;
    }
}
