//! `FadeEffect` — standalone gain ramp applied after volume.
//!
//! Used by `FlowController` for crossfade fade-in / fade-out gain on the
//! current track.

use crate::audio::constants::{INT16_MAX_F, INT16_MIN_F};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FadeCurve {
    Linear,
    Sinusoidal,
}

impl FadeCurve {
    fn value(self, t: f32) -> f32 {
        match self {
            FadeCurve::Linear => t,
            FadeCurve::Sinusoidal => 0.5 * (1.0 - (t * std::f32::consts::PI).cos()),
        }
    }
}

pub struct FadeEffect {
    /// Instantaneous gain applied to the output.
    current_gain: f32,
    target_gain: f32,
    start_gain: f32,
    fade_samples_total: usize,
    fade_samples_elapsed: usize,
    fade_active: bool,
    curve: FadeCurve,
    _channels: usize,
}

impl FadeEffect {
    pub fn new(initial_gain: f32, channels: usize) -> Self {
        Self {
            current_gain: initial_gain,
            target_gain: initial_gain,
            start_gain: initial_gain,
            fade_samples_total: 0,
            fade_samples_elapsed: 0,
            fade_active: false,
            curve: FadeCurve::Sinusoidal,
            _channels: channels,
        }
    }

    /// Set gain immediately (no interpolation).
    pub fn set_gain(&mut self, gain: f32) {
        self.current_gain = gain;
        self.target_gain = gain;
        self.start_gain = gain;
        self.fade_active = false;
    }

    /// Schedule a gain ramp from `current_gain` → `target` over `duration_ms`.
    pub fn fade_to(&mut self, target: f32, duration_ms: u64, curve: FadeCurve, sample_rate: u32) {
        if duration_ms == 0 {
            self.set_gain(target);
            return;
        }
        self.start_gain = self.current_gain;
        self.target_gain = target;
        self.fade_samples_total = (sample_rate as u64 * duration_ms / 1000) as usize;
        self.fade_samples_elapsed = 0;
        self.fade_active = self.fade_samples_total > 0;
        self.curve = curve;
    }

    pub fn current_gain(&self) -> f32 {
        self.current_gain
    }

    pub fn is_done(&self) -> bool {
        !self.fade_active
    }

    /// Process `frame` in-place.
    pub fn process(&mut self, frame: &mut [i16]) {
        let sample_count = frame.len();
        if sample_count == 0 {
            return;
        }

        // Short-circuit: no fade, gain == 1.0 → nothing to do.
        if !self.fade_active && (self.current_gain - 1.0).abs() < f32::EPSILON {
            return;
        }

        let (gain_start, gain_end) = if self.fade_active && self.fade_samples_total > 0 {
            let prev = self.fade_samples_elapsed;
            let next = (prev + sample_count).min(self.fade_samples_total);

            let t0 = prev as f32 / self.fade_samples_total as f32;
            let t1 = next as f32 / self.fade_samples_total as f32;

            let range = self.target_gain - self.start_gain;
            let gs = self.start_gain + range * self.curve.value(t0);
            let ge = self.start_gain + range * self.curve.value(t1);

            self.fade_samples_elapsed = next;
            if next >= self.fade_samples_total {
                self.fade_active = false;
                self.current_gain = self.target_gain;
            } else {
                self.current_gain = ge;
            }
            (gs, ge)
        } else {
            let g = self.current_gain;
            (g, g)
        };

        let step = if sample_count > 1 {
            (gain_end - gain_start) / (sample_count - 1) as f32
        } else {
            0.0
        };
        let mut gain = gain_start;

        for s in frame.iter_mut() {
            let out = (*s as f32 * gain).clamp(INT16_MIN_F, INT16_MAX_F);
            *s = out.round() as i16;
            gain += step;
        }
    }
}
