use super::AudioFilter;

/// Volume filter — scales all samples by a float factor.
pub struct VolumeFilter {
  volume: f32,
}

impl VolumeFilter {
  pub fn new(volume: f32) -> Self {
    Self { volume }
  }
}

impl AudioFilter for VolumeFilter {
  fn process(&mut self, samples: &mut [i16]) {
    let vol = self.volume;
    for sample in samples.iter_mut() {
      let s = (*sample as f32 * vol) as i32;
      *sample = s.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
    }
  }

  fn is_enabled(&self) -> bool {
    (self.volume - 1.0).abs() > f32::EPSILON
  }

  fn reset(&mut self) {}
}
