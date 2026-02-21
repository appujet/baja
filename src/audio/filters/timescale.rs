use super::AudioFilter;

/// 4-point Catmull-Rom (Hermite) cubic interpolation.
fn cubic_interpolate(p0: f64, p1: f64, p2: f64, p3: f64, t: f64) -> f64 {
  let t2 = t * t;
  let t3 = t2 * t;

  0.5
    * (2.0 * p1
      + (-p0 + p2) * t
      + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2
      + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t3)
}

/// Timescale filter — speed/pitch/rate via cubic interpolation resampling.
///
/// This filter changes the number of output samples. The `process` method
/// resamples in-place as much as possible and returns the actual number of
/// valid output samples via `process_resample`.
pub struct TimescaleFilter {
  _speed: f64,
  _pitch: f64,
  _rate: f64,
  final_rate: f64,
  /// Leftover input from previous calls
  input_buffer: Vec<i16>,
}

impl TimescaleFilter {
  pub fn new(speed: f64, pitch: f64, rate: f64) -> Self {
    let speed = if speed < 0.1 {
      0.1
    } else if speed > 5.0 {
      5.0
    } else {
      speed
    };
    let pitch = if pitch < 0.1 {
      0.1
    } else if pitch > 5.0 {
      5.0
    } else {
      pitch
    };
    let rate = if rate < 0.1 {
      0.1
    } else if rate > 5.0 {
      5.0
    } else {
      rate
    };
    let final_rate = speed * pitch * rate;

    Self {
      _speed: speed,
      _pitch: pitch,
      _rate: rate,
      final_rate,
      input_buffer: Vec::new(),
    }
  }

  /// Process with resampling. Returns a new buffer (may differ in length from input).
  pub fn process_resample(&mut self, samples: &[i16]) -> Vec<i16> {
    if self.final_rate == 1.0 {
      return samples.to_vec();
    }

    if self.final_rate == 0.0 {
      return Vec::new();
    }

    self.input_buffer.extend_from_slice(samples);

    // Need at least 4 stereo frames for cubic interpolation
    if self.input_buffer.len() < 16 {
      return Vec::new();
    }

    // The input buffer is interleaved stereo: [L0, R0, L1, R1, ...]
    // Number of stereo frames:
    let num_input_frames = self.input_buffer.len() / 2;

    // Estimate output frames
    let output_frames = (num_input_frames as f64 / self.final_rate) as usize;
    let mut output = Vec::with_capacity(output_frames * 2);

    let mut output_frame = 0usize;
    loop {
      let input_frame_f = output_frame as f64 * self.final_rate;
      let i1 = input_frame_f as usize;
      let frac = input_frame_f - i1 as f64;

      let p3_idx = i1 + 2;
      if p3_idx >= num_input_frames {
        break;
      }

      let p0_idx = if i1 == 0 { 0 } else { i1 - 1 };
      let p1_idx = i1;
      let p2_idx = i1 + 1;

      // Left channel
      let p0_l = self.input_buffer[p0_idx * 2] as f64;
      let p1_l = self.input_buffer[p1_idx * 2] as f64;
      let p2_l = self.input_buffer[p2_idx * 2] as f64;
      let p3_l = self.input_buffer[p3_idx * 2] as f64;
      let out_l = cubic_interpolate(p0_l, p1_l, p2_l, p3_l, frac);
      output.push((out_l as i32).clamp(i16::MIN as i32, i16::MAX as i32) as i16);

      // Right channel
      let p0_r = self.input_buffer[p0_idx * 2 + 1] as f64;
      let p1_r = self.input_buffer[p1_idx * 2 + 1] as f64;
      let p2_r = self.input_buffer[p2_idx * 2 + 1] as f64;
      let p3_r = self.input_buffer[p3_idx * 2 + 1] as f64;
      let out_r = cubic_interpolate(p0_r, p1_r, p2_r, p3_r, frac);
      output.push((out_r as i32).clamp(i16::MIN as i32, i16::MAX as i32) as i16);

      output_frame += 1;
    }

    // Consume the input frames we used
    let consumed_frames = (output_frame as f64 * self.final_rate) as usize;
    let consumed_samples = consumed_frames * 2;
    if consumed_samples < self.input_buffer.len() {
      self.input_buffer = self.input_buffer[consumed_samples..].to_vec();
    } else {
      self.input_buffer.clear();
    }

    output
  }
}

impl AudioFilter for TimescaleFilter {
  fn process(&mut self, _samples: &mut [i16]) {
    // Timescale cannot work in-place because it changes buffer length.
    // Use `process_resample` instead. This is a no-op for the in-place trait.
  }

  fn is_enabled(&self) -> bool {
    (self.final_rate - 1.0).abs() > f64::EPSILON
  }

  fn reset(&mut self) {
    self.input_buffer.clear();
  }
}
