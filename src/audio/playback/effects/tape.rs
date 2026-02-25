//! `TapeEffect` — high-quality tape start/stop resampling.
//!
//! Mirrors NodeLink's `TapeTransformer.ts`. Uses Cubic Hermite Spline
//! (Catmull-Rom) interpolation for smooth pitch/speed ramps.

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TapeCurve {
    Linear,
    Exponential,
    Sinusoidal,
}

impl TapeCurve {
    fn value(self, t: f32) -> f32 {
        match self {
            Self::Linear => t,
            Self::Exponential => t * t,
            Self::Sinusoidal => 0.5 * (1.0 - (t * std::f32::consts::PI).cos()),
        }
    }
}

struct TapeState {
    start_rate: f32,
    target_rate: f32,
    duration_ms: f32,
    elapsed_ms: f32,
    curve: TapeCurve,
}

pub struct TapeEffect {
    sample_rate: u32,
    channels: usize,
    current_rate: f32,
    tape: Option<TapeState>,
    ramp_completed: bool,

    // Resampling buffer
    input_buffer: Vec<f32>,
    read_pos: f64,
}

impl TapeEffect {
    pub fn new(sample_rate: u32, channels: usize) -> Self {
        // 10 second buffer
        let max_size = (sample_rate as usize * channels * 10).max(96000);
        Self {
            sample_rate,
            channels,
            current_rate: 1.0,
            tape: None,
            ramp_completed: false,
            input_buffer: Vec::with_capacity(max_size),
            read_pos: 0.0,
        }
    }

    pub fn set_rate(&mut self, rate: f32) {
        self.current_rate = rate.max(0.01).min(2.0);
        self.tape = None;
        self.ramp_completed = false;
    }

    pub fn tape_to(&mut self, duration_ms: f32, variant: &str, curve: &str) {
        let target_rate = if variant == "start" { 1.0 } else { 0.01 };
        let curve_type = match curve {
            "linear" => TapeCurve::Linear,
            "exponential" => TapeCurve::Exponential,
            _ => TapeCurve::Sinusoidal,
        };

        if duration_ms <= 0.0 {
            self.current_rate = target_rate;
            self.tape = None;
            return;
        }

        self.tape = Some(TapeState {
            start_rate: self.current_rate,
            target_rate,
            duration_ms,
            elapsed_ms: 0.0,
            curve: curve_type,
        });
        self.ramp_completed = false;
    }

    pub fn is_active(&self) -> bool {
        self.tape.is_some() || (self.current_rate - 1.0).abs() > 0.001
    }

    pub fn check_ramp_completed(&mut self) -> bool {
        if self.ramp_completed {
            self.ramp_completed = false;
            true
        } else {
            false
        }
    }

    /// Process a PCM frame. Unlike the previous implementation which worked in-place,
    /// this uses an internal resampling buffer to handle rate changes correctly.
    pub fn process(&mut self, frame: &mut [i16]) {
        if frame.is_empty() {
            return;
        }

        let channels = self.channels;

        // Push current frame to input float buffer
        for &s in frame.iter() {
            self.input_buffer.push(s as f32 / 32767.0);
        }

        let mut out_idx = 0;
        let sample_duration_ms = 1000.0 / self.sample_rate as f32;

        while out_idx < frame.len() {
            // Update rate ramp
            if let Some(state) = &mut self.tape {
                state.elapsed_ms += sample_duration_ms;
                let t = (state.elapsed_ms / state.duration_ms).min(1.0);
                let curve_t = state.curve.value(t);
                self.current_rate =
                    state.start_rate + (state.target_rate - state.start_rate) * curve_t;

                if t >= 1.0 {
                    self.current_rate = state.target_rate;
                    self.tape = None;
                    self.ramp_completed = true;
                }
            }

            if self.current_rate <= 0.01 && self.tape.is_none() {
                // Silenced
                while out_idx < frame.len() {
                    frame[out_idx] = 0;
                    out_idx += 1;
                }
                break;
            }

            // Cubic Hermite Spline Interpolation
            let i_pos = (self.read_pos.floor() as usize / channels) * channels;
            if i_pos + channels * 3 >= self.input_buffer.len() {
                // Not enough data to interpolate, fill remainder with zero and break
                while out_idx < frame.len() {
                    frame[out_idx] = 0;
                    out_idx += 1;
                }
                break;
            }

            let frac = ((self.read_pos - i_pos as f64) / channels as f64) as f32;

            for c in 0..channels {
                let p0 = if i_pos >= channels {
                    self.input_buffer[i_pos - channels + c]
                } else {
                    self.input_buffer[i_pos + c]
                };
                let p1 = self.input_buffer[i_pos + c];
                let p2 = self.input_buffer[i_pos + channels + c];
                let p3 = self.input_buffer[i_pos + channels * 2 + c];

                // Catmull-Rom
                let val = 0.5
                    * (2.0 * p1
                        + (-p0 + p2) * frac
                        + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * frac * frac
                        + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * frac * frac * frac);

                if out_idx < frame.len() {
                    frame[out_idx] = (val * 32767.0).clamp(-32768.0, 32767.0).round() as i16;
                    out_idx += 1;
                }
            }

            self.read_pos += self.current_rate as f64 * channels as f64;
        }

        // Compact buffer
        if self.read_pos > (self.sample_rate as f64 * channels as f64 * 2.0) {
            let integral = (self.read_pos.floor() as usize / channels) * channels;
            self.input_buffer.drain(0..integral);
            self.read_pos -= integral as f64;
        }
    }
}
