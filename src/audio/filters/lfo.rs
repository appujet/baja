use std::f64::consts::PI;

const SAMPLE_RATE: f64 = 48000.0;
const TWO_PI: f64 = 2.0 * PI;

/// Low-Frequency Oscillator (sine wave) used by tremolo, vibrato, and rotation filters.
pub struct Lfo {
    phase: f64,
    pub frequency: f64,
    pub depth: f64,
}

impl Lfo {
    pub fn new() -> Self {
        Self {
            phase: 0.0,
            frequency: 0.0,
            depth: 0.0,
        }
    }

    pub fn update(&mut self, frequency: f64, depth: f64) {
        self.frequency = frequency;
        self.depth = depth;
    }

    /// Returns the raw sine wave value in [-1, 1] and advances the phase.
    pub fn get_value(&mut self) -> f64 {
        if self.frequency == 0.0 {
            return 0.0;
        }
        let value = self.phase.sin();
        self.phase += TWO_PI * self.frequency / SAMPLE_RATE;
        if self.phase > TWO_PI {
            self.phase -= TWO_PI;
        }
        value
    }

    /// Returns an amplitude multiplier for tremolo: `1.0 - depth * (sin+1)/2`.
    pub fn process(&mut self) -> f64 {
        if self.depth == 0.0 || self.frequency == 0.0 {
            return 1.0;
        }
        let lfo_value = self.get_value();
        let normalized = (lfo_value + 1.0) / 2.0;
        1.0 - self.depth * normalized
    }

    pub fn reset(&mut self) {
        self.phase = 0.0;
    }
}
