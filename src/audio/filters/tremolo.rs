use super::AudioFilter;
use super::lfo::Lfo;

/// Tremolo filter — LFO-driven amplitude modulation.
pub struct TremoloFilter {
    lfo: Lfo,
}

impl TremoloFilter {
    pub fn new(frequency: f32, depth: f32) -> Self {
        let mut lfo = Lfo::new();
        let depth = depth.clamp(0.0, 1.0);
        lfo.update(frequency as f64, depth as f64);
        Self { lfo }
    }
}

impl AudioFilter for TremoloFilter {
    fn process(&mut self, samples: &mut [i16]) {
        if self.lfo.depth == 0.0 || self.lfo.frequency == 0.0 {
            return;
        }

        // Process per-sample (both L and R get the same multiplier per sample)
        for sample in samples.iter_mut() {
            let multiplier = self.lfo.process();
            let s = (*sample as f64 * multiplier) as i32;
            *sample = s.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        }
    }

    fn is_enabled(&self) -> bool {
        self.lfo.depth > 0.0 && self.lfo.frequency > 0.0
    }

    fn reset(&mut self) {
        self.lfo.reset();
    }
}
