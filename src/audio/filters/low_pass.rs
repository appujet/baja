use super::AudioFilter;

/// Low-pass filter — IIR single-pole smoothing. Matches Lavalink smoothing parameter.
pub struct LowPassFilter {
  smoothing: f32,
  smoothing_factor: f64,
  prev_left: f64,
  prev_right: f64,
}

impl LowPassFilter {
  pub fn new(smoothing: f32) -> Self {
    let smoothing_factor = if smoothing > 1.0 {
      1.0 / smoothing as f64
    } else {
      0.0
    };
    Self {
      smoothing,
      smoothing_factor,
      prev_left: 0.0,
      prev_right: 0.0,
    }
  }
}

impl AudioFilter for LowPassFilter {
  fn process(&mut self, samples: &mut [i16]) {
    if self.smoothing <= 1.0 {
      return;
    }

    let num_frames = samples.len() / 2;
    for frame in 0..num_frames {
      let offset = frame * 2;

      let left = samples[offset] as f64;
      let new_left = self.prev_left + self.smoothing_factor * (left - self.prev_left);
      self.prev_left = new_left;
      samples[offset] = (new_left as i32).clamp(i16::MIN as i32, i16::MAX as i32) as i16;

      let right = samples[offset + 1] as f64;
      let new_right = self.prev_right + self.smoothing_factor * (right - self.prev_right);
      self.prev_right = new_right;
      samples[offset + 1] = (new_right as i32).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
    }
  }

  fn is_enabled(&self) -> bool {
    self.smoothing > 1.0
  }

  fn reset(&mut self) {
    self.prev_left = 0.0;
    self.prev_right = 0.0;
  }
}
