use super::AudioFilter;

fn cubic_resample(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> f32 {
    let t2 = t * t;
    let t3 = t2 * t;

    0.5 * (2.0 * p1
        + (-p0 + p2) * t
        + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2
        + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t3)
}

pub struct TimescaleFilter {
    _speed: f64,
    _pitch: f64,
    _rate: f64,
    final_rate: f32,
    input_buffer: Vec<i16>,
    position: f32,
}

impl TimescaleFilter {
    pub fn new(speed: f64, pitch: f64, rate: f64) -> Self {
        let speed = speed.clamp(0.1, 5.0);
        let pitch = pitch.clamp(0.1, 5.0);
        let rate = rate.clamp(0.1, 5.0);
        let final_rate = (speed * pitch * rate) as f32;

        Self {
            _speed: speed,
            _pitch: pitch,
            _rate: rate,
            final_rate,
            input_buffer: Vec::with_capacity(4096),
            position: 0.0,
        }
    }

    pub fn process_resample(&mut self, samples: &[i16]) -> Vec<i16> {
        if (self.final_rate - 1.0).abs() < f32::EPSILON {
            return samples.to_vec();
        }

        if self.final_rate <= 0.0 {
            return Vec::new();
        }

        self.input_buffer.extend_from_slice(samples);

        let num_input_samples = self.input_buffer.len();
        let num_input_frames = num_input_samples / 2;

        if num_input_frames < 4 {
            return Vec::new();
        }

        let output_frames_est = (num_input_frames as f32 / self.final_rate) as usize + 2;
        let mut output = Vec::with_capacity(output_frames_est * 2);

        while (self.position as usize) + 2 < num_input_frames {
            let i1 = self.position as usize;
            let frac = self.position - i1 as f32;

            let p0_idx = i1.saturating_sub(1);
            let p1_idx = i1;
            let p2_idx = i1 + 1;
            let p3_idx = i1 + 2;

            // Left channel
            let p0_l = self.input_buffer[p0_idx * 2] as f32 / 32768.0;
            let p1_l = self.input_buffer[p1_idx * 2] as f32 / 32768.0;
            let p2_l = self.input_buffer[p2_idx * 2] as f32 / 32768.0;
            let p3_l = self.input_buffer[p3_idx * 2] as f32 / 32768.0;
            let out_l = cubic_resample(p0_l, p1_l, p2_l, p3_l, frac);
            output.push((out_l.clamp(-1.0, 1.0) * 32767.0) as i16);

            // Right channel
            let p0_r = self.input_buffer[p0_idx * 2 + 1] as f32 / 32768.0;
            let p1_r = self.input_buffer[p1_idx * 2 + 1] as f32 / 32768.0;
            let p2_r = self.input_buffer[p2_idx * 2 + 1] as f32 / 32768.0;
            let p3_r = self.input_buffer[p3_idx * 2 + 1] as f32 / 32768.0;
            let out_r = cubic_resample(p0_r, p1_r, p2_r, p3_r, frac);
            output.push((out_r.clamp(-1.0, 1.0) * 32767.0) as i16);

            self.position += self.final_rate;
        }

        let consumed_frames = self.position.floor() as usize;
        let keep_from_frame = consumed_frames.saturating_sub(1);

        if keep_from_frame > 0 {
            let samples_to_drain = keep_from_frame * 2;
            if samples_to_drain < self.input_buffer.len() {
                self.input_buffer.drain(0..samples_to_drain);
                self.position -= keep_from_frame as f32;
            }
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
        (self.final_rate - 1.0).abs() > f32::EPSILON
    }

    fn reset(&mut self) {
        self.input_buffer.clear();
        self.position = 0.0;
    }
}
