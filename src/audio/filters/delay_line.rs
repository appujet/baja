/// Circular buffer delay line used by the vibrato filter.
pub struct DelayLine {
  buffer: Vec<f32>,
  size: usize,
  write_index: usize,
}

impl DelayLine {
  pub fn new(size: usize) -> Self {
    Self {
      buffer: vec![0.0; size],
      size,
      write_index: 0,
    }
  }

  pub fn write(&mut self, sample: f32) {
    self.buffer[self.write_index] = sample;
    self.write_index = (self.write_index + 1) % self.size;
  }

  pub fn read(&self, delay_in_samples: f64) -> f32 {
    let safe_delay = delay_in_samples.max(0.0).min((self.size - 1) as f64);
    let int_delay = safe_delay as usize;
    let frac = safe_delay - int_delay as f64;

    let idx0 = (self.write_index + self.size - int_delay) % self.size;
    let idx1 = (self.write_index + self.size - int_delay - 1) % self.size;

    // Linear interpolation between adjacent samples for smooth delay
    let s0 = self.buffer[idx0] as f64;
    let s1 = self.buffer[idx1] as f64;
    (s0 * (1.0 - frac) + s1 * frac) as f32
  }

  pub fn clear(&mut self) {
    self.buffer.fill(0.0);
  }
}
