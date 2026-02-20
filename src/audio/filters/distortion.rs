use super::AudioFilter;

const MAX_INT_16: f64 = 32767.0;

/// Distortion filter — wave shaping via sin/cos/tan with configurable offsets and scales.
pub struct DistortionFilter {
    sin_offset: f32,
    sin_scale: f32,
    cos_offset: f32,
    cos_scale: f32,
    tan_offset: f32,
    tan_scale: f32,
    offset: f32,
    scale: f32,
}

impl DistortionFilter {
    pub fn new(
        sin_offset: f32,
        sin_scale: f32,
        cos_offset: f32,
        cos_scale: f32,
        tan_offset: f32,
        tan_scale: f32,
        offset: f32,
        scale: f32,
    ) -> Self {
        Self {
            sin_offset,
            sin_scale,
            cos_offset,
            cos_scale,
            tan_offset,
            tan_scale,
            offset,
            scale,
        }
    }
}

impl AudioFilter for DistortionFilter {
    fn process(&mut self, samples: &mut [i16]) {
        let num_frames = samples.len() / 2;

        for frame in 0..num_frames {
            let offset_idx = frame * 2;

            for ch in 0..2 {
                let sample = samples[offset_idx + ch] as f64;
                let normalized = sample / MAX_INT_16;

                let mut distorted = 0.0f64;

                if self.sin_scale != 0.0 {
                    distorted +=
                        (normalized * self.sin_scale as f64 + self.sin_offset as f64).sin();
                }

                if self.cos_scale != 0.0 {
                    distorted +=
                        (normalized * self.cos_scale as f64 + self.cos_offset as f64).cos();
                }

                if self.tan_scale != 0.0 {
                    let tan_input = (normalized * self.tan_scale as f64 + self.tan_offset as f64)
                        .clamp(
                            -std::f64::consts::FRAC_PI_2 + 0.01,
                            std::f64::consts::FRAC_PI_2 - 0.01,
                        );
                    distorted += tan_input.tan();
                }

                distorted = (distorted * self.scale as f64 + self.offset as f64) * MAX_INT_16;
                samples[offset_idx + ch] =
                    (distorted as i32).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
            }
        }
    }

    fn is_enabled(&self) -> bool {
        self.sin_offset != 0.0
            || self.sin_scale != 0.0
            || self.cos_offset != 0.0
            || self.cos_scale != 0.0
            || self.tan_offset != 0.0
            || self.tan_scale != 0.0
            || self.offset != 0.0
            || (self.scale - 1.0).abs() > f32::EPSILON
    }

    fn reset(&mut self) {}
}
