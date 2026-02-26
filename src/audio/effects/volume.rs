//! `VolumeEffect` — per-sample gain with sinusoidal fade transitions and a
//! soft exponential limiter.
//!
//! Mirrors NodeLink's `VolumeTransformer.ts` (without AGC for simplicity;
//! the soft limiter alone prevents clipping).

use crate::audio::constants::{INT16_MAX_F, INT16_MIN_F};

/// Fade curve shapes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FadeCurve {
    Linear,
    Sinusoidal,
}

/// Per-frame volume processor with soft limiter and sinusoidal fade support.
pub struct VolumeEffect {
    /// Currently applied gain (tracks toward `target_volume` during a fade).
    current_volume: f32,
    target_volume: f32,
    start_volume: f32,

    // Fade state
    fade_frames_total: usize,
    fade_frames_elapsed: usize,
    fade_active: bool,
    fade_curve: FadeCurve,

    // Soft limiter parameters
    _limiter_threshold: f32, // 0.0 – 1.0 relative to INT16_MAX
    limiter_softness: f32,

    // Computed from threshold
    threshold_value: f32,
    limit_headroom: f32,

    _sample_rate: u32,
    channels: usize,
}

impl VolumeEffect {
    pub fn new(volume: f32, sample_rate: u32, channels: usize) -> Self {
        let limiter_threshold = 0.95_f32;
        let limiter_softness = 0.4_f32;
        let threshold_value = limiter_threshold * INT16_MAX_F;
        let limit_headroom = INT16_MAX_F - threshold_value;

        // 1000 ms fade duration by default
        let fade_frames_total = (sample_rate as usize * 1000) / 1000; // 1 s

        Self {
            current_volume: volume,
            target_volume: volume,
            start_volume: volume,
            fade_frames_total,
            fade_frames_elapsed: fade_frames_total, // start "done"
            fade_active: false,
            fade_curve: FadeCurve::Sinusoidal,
            _limiter_threshold: limiter_threshold,
            limiter_softness,
            threshold_value,
            limit_headroom,
            _sample_rate: sample_rate,
            channels,
        }
    }

    /// Set the target volume.  Triggers a sinusoidal fade from current → target.
    pub fn set_volume(&mut self, volume: f32) {
        if (volume - self.target_volume).abs() < f32::EPSILON {
            return;
        }
        self.start_volume = self.current_volume;
        self.target_volume = volume;
        self.fade_frames_elapsed = 0;
        self.fade_active = self.fade_frames_total > 0;
        if !self.fade_active {
            self.current_volume = volume;
        }
    }

    /// Get current gain (after any ongoing fade step).
    pub fn current_volume(&self) -> f32 {
        self.current_volume
    }

    fn curve_value(&self, t: f32) -> f32 {
        match self.fade_curve {
            FadeCurve::Linear => t,
            FadeCurve::Sinusoidal => 0.5 * (1.0 - (t * std::f32::consts::PI).cos()),
        }
    }

    fn apply_limiter(&self, value: f32) -> f32 {
        let abs = value.abs();
        if abs <= self.threshold_value || self.limit_headroom <= 0.0 {
            return value;
        }
        let overshoot = (abs - self.threshold_value) / self.limit_headroom;
        let softened = 1.0 - (-overshoot * self.limiter_softness).exp();
        let limited = self.threshold_value + self.limit_headroom * softened;
        value.signum() * limited.min(INT16_MAX_F)
    }

    /// Process a stereo interleaved i16 frame **in-place**.
    pub fn process(&mut self, frame: &mut [i16]) {
        let sample_count = frame.len();
        if sample_count == 0 {
            return;
        }

        // Compute per-call gain range (start → end)
        let (gain_start, gain_end) = if self.fade_active && self.fade_frames_total > 0 {
            let frames = sample_count / self.channels;
            let prev = self.fade_frames_elapsed;
            let next = (prev + frames).min(self.fade_frames_total);

            let t_start = prev as f32 / self.fade_frames_total as f32;
            let t_end = next as f32 / self.fade_frames_total as f32;

            let range = self.target_volume - self.start_volume;
            let gs = self.start_volume + range * self.curve_value(t_start);
            let ge = self.start_volume + range * self.curve_value(t_end);

            self.fade_frames_elapsed = next;
            if next >= self.fade_frames_total {
                self.fade_active = false;
                self.current_volume = self.target_volume;
            } else {
                self.current_volume = ge;
            }
            (gs, ge)
        } else {
            let v = self.target_volume;
            (v, v)
        };

        let step = if sample_count > 1 {
            (gain_end - gain_start) / (sample_count - 1) as f32
        } else {
            0.0
        };
        let mut gain = gain_start;

        for s in frame.iter_mut() {
            let scaled = *s as f32 * gain;
            let limited = self.apply_limiter(scaled);
            *s = limited.clamp(INT16_MIN_F, INT16_MAX_F).round() as i16;
            gain += step;
        }
    }
}
