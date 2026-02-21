use std::f64::consts::PI;

/// Biquad filter coefficients (normalized by a0).
#[derive(Clone)]
pub struct BiquadCoeffs {
  pub b0: f64,
  pub b1: f64,
  pub b2: f64,
  pub a1: f64,
  pub a2: f64,
}

/// Per-channel biquad filter state (Direct Form I).
#[derive(Clone, Default)]
pub struct BiquadState {
  pub x1: f64,
  pub x2: f64,
  pub y1: f64,
  pub y2: f64,
}

impl BiquadCoeffs {
  /// Bandpass filter for equalizer bands.
  pub fn bandpass(freq: f64, q: f64, sample_rate: f64) -> Self {
    let omega0 = 2.0 * PI * freq / sample_rate;
    let sin_omega0 = omega0.sin();
    let cos_omega0 = omega0.cos();
    let alpha = sin_omega0 / (2.0 * q);

    let a0 = 1.0 + alpha;
    Self {
      b0: alpha / a0,
      b1: 0.0,
      b2: -alpha / a0,
      a1: -2.0 * cos_omega0 / a0,
      a2: (1.0 - alpha) / a0,
    }
  }

  /// Low-pass filter for karaoke.
  pub fn lowpass(freq: f64, q: f64, sample_rate: f64) -> Self {
    let omega0 = 2.0 * PI * freq / sample_rate;
    let sin_omega0 = omega0.sin();
    let cos_omega0 = omega0.cos();
    let alpha = sin_omega0 / (2.0 * q);

    let a0 = 1.0 + alpha;
    let inv_a0 = 1.0 / a0;
    Self {
      b0: (1.0 - cos_omega0) * 0.5 * inv_a0,
      b1: (1.0 - cos_omega0) * inv_a0,
      b2: (1.0 - cos_omega0) * 0.5 * inv_a0,
      a1: -2.0 * cos_omega0 * inv_a0,
      a2: (1.0 - alpha) * inv_a0,
    }
  }

  /// High-pass filter for karaoke.
  pub fn highpass(freq: f64, q: f64, sample_rate: f64) -> Self {
    let omega0 = 2.0 * PI * freq / sample_rate;
    let sin_omega0 = omega0.sin();
    let cos_omega0 = omega0.cos();
    let alpha = sin_omega0 / (2.0 * q);

    let a0 = 1.0 + alpha;
    let inv_a0 = 1.0 / a0;
    Self {
      b0: (1.0 + cos_omega0) * 0.5 * inv_a0,
      b1: -(1.0 + cos_omega0) * inv_a0,
      b2: (1.0 + cos_omega0) * 0.5 * inv_a0,
      a1: -2.0 * cos_omega0 * inv_a0,
      a2: (1.0 - alpha) * inv_a0,
    }
  }
}

impl BiquadState {
  /// Process one sample through the biquad filter.
  pub fn process(&mut self, input: f64, coeffs: &BiquadCoeffs) -> f64 {
    let output = coeffs.b0 * input + coeffs.b1 * self.x1 + coeffs.b2 * self.x2
      - coeffs.a1 * self.y1
      - coeffs.a2 * self.y2;

    if !output.is_finite() {
      // Reset on NaN/Inf to avoid cascading errors
      self.x1 = 0.0;
      self.x2 = 0.0;
      self.y1 = 0.0;
      self.y2 = 0.0;
      return 0.0;
    }

    self.x2 = self.x1;
    self.x1 = input;
    self.y2 = self.y1;
    self.y1 = output;
    output
  }

  pub fn reset(&mut self) {
    self.x1 = 0.0;
    self.x2 = 0.0;
    self.y1 = 0.0;
    self.y2 = 0.0;
  }
}
