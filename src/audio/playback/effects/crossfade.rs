//! `CrossfadeController` — constant-power track crossfading.
//!
//! Mirrors NodeLink's `CrossfadeController.ts`. Buffers the next track's PCM
//! into a `RingBuffer` ahead of time, then blends it with the main track
//! using cos/sin curves to maintain perceived loudness.

use super::fade::FadeCurve;
use crate::audio::buffer::PooledBuffer;
use crate::audio::ring_buffer::RingBuffer;
use flume::Receiver;

const HALF_PI: f32 = std::f32::consts::PI / 2.0;
const INT16_MAX_F: f32 = 32767.0;
const INT16_MIN_F: f32 = -32768.0;

pub struct CrossfadeController {
    sample_rate: u32,
    channels: usize,
    bytes_per_ms: usize,

    // Buffering state
    ring_buffer: Option<RingBuffer>,
    next_rx: Option<Receiver<PooledBuffer>>,

    // Fade state
    active_fade: Option<CrossfadeState>,
    target_buffer_bytes: usize,
}

struct CrossfadeState {
    duration_ms: u64,
    elapsed_ms: f32,
    curve: FadeCurve,
}

impl CrossfadeController {
    pub fn new(sample_rate: u32, channels: usize) -> Self {
        let bytes_per_ms = (sample_rate as usize * channels * 2) / 1000;
        Self {
            sample_rate,
            channels,
            bytes_per_ms,
            ring_buffer: None,
            next_rx: None,
            active_fade: None,
            target_buffer_bytes: 0,
        }
    }

    /// Prepare the next track for crossfading by attaching its PCM receiver.
    /// `duration_ms` determines how much audio is buffered ahead of time.
    pub fn prepare(&mut self, rx: Receiver<PooledBuffer>, duration_ms: u64) {
        self.clear();
        let buffer_size = (duration_ms as usize * self.bytes_per_ms).max(8192);
        self.ring_buffer = Some(RingBuffer::new(buffer_size));
        self.target_buffer_bytes = buffer_size;
        self.next_rx = Some(rx);
    }

    /// Pull as much data as possible from the next track's receiver into the buffer.
    pub fn fill_buffer(&mut self) {
        let Some(rx) = &self.next_rx else { return };
        let Some(ring) = &mut self.ring_buffer else {
            return;
        };

        // Drain pending frames into the ring buffer
        while !rx.is_empty() {
            if let Ok(pooled) = rx.try_recv() {
                // Convert i16 slice to u8 slice safely
                let bytes = unsafe {
                    std::slice::from_raw_parts(pooled.as_ptr() as *const u8, pooled.len() * 2)
                };
                ring.write(bytes);
            } else {
                break;
            }
        }
    }

    pub fn is_ready(&self) -> bool {
        let Some(ring) = &self.ring_buffer else {
            return false;
        };
        // Ready if we have at least 80% of target buffered or at least 1s
        ring.len()
            >= (self.target_buffer_bytes * 8 / 10)
                .min(self.sample_rate as usize * self.channels * 2)
    }

    pub fn start_crossfade(&mut self, duration_ms: u64, curve: FadeCurve) -> bool {
        if self.ring_buffer.is_none() || !self.is_ready() {
            return false;
        }
        self.active_fade = Some(CrossfadeState {
            duration_ms,
            elapsed_ms: 0.0,
            curve,
        });
        true
    }

    pub fn is_active(&self) -> bool {
        self.active_fade.is_some()
    }

    pub fn clear(&mut self) {
        self.ring_buffer = None; // Drop returns buffer to pool
        self.next_rx = None;
        self.active_fade = None;
        self.target_buffer_bytes = 0;
    }

    /// Mix the buffered next track into `frame` if crossfade is active.
    /// Returns true if the fade finished during this call.
    pub fn process(&mut self, frame: &mut [i16]) -> bool {
        let (elapsed, duration, curve) = match &self.active_fade {
            Some(s) => (s.elapsed_ms, s.duration_ms as f32, s.curve),
            None => return false,
        };

        let sample_count = frame.len();
        let byte_count = sample_count * 2;

        let next_bytes = if let Some(ring) = &mut self.ring_buffer {
            ring.read(byte_count)
        } else {
            return false;
        };

        let Some(next_bytes) = next_bytes else {
            // No buffered data available?? stall or skip
            return false;
        };

        // Safety: cast u8 vec back to i16 slice
        let next_samples = unsafe {
            std::slice::from_raw_parts(next_bytes.as_ptr() as *const i16, next_bytes.len() / 2)
        };

        let chunk_ms =
            (sample_count as f32 / self.channels as f32 / self.sample_rate as f32) * 1000.0;

        let t_start = (elapsed / duration).min(1.0);
        let t_end = ((elapsed + chunk_ms) / duration).min(1.0);

        let (out_start, in_start) = self.fade_gains(t_start, curve);
        let (out_end, in_end) = self.fade_gains(t_end, curve);

        let step_out = if sample_count > 1 {
            (out_end - out_start) / (sample_count - 1) as f32
        } else {
            0.0
        };
        let step_in = if sample_count > 1 {
            (in_end - in_start) / (sample_count - 1) as f32
        } else {
            0.0
        };

        let mut g_out = out_start;
        let mut g_in = in_start;

        for i in 0..sample_count {
            let next_val = next_samples.get(i).copied().unwrap_or(0);
            let mixed = (frame[i] as f32 * g_out) + (next_val as f32 * g_in);
            frame[i] = mixed.clamp(INT16_MIN_F, INT16_MAX_F).round() as i16;
            g_out += step_out;
            g_in += step_in;
        }

        let finished = if let Some(state) = &mut self.active_fade {
            state.elapsed_ms += chunk_ms;
            state.elapsed_ms >= state.duration_ms as f32
        } else {
            false
        };
        if finished {
            self.active_fade = None;
        }
        finished
    }

    fn fade_gains(&self, t: f32, curve: FadeCurve) -> (f32, f32) {
        let t = t.clamp(0.0, 1.0);
        match curve {
            FadeCurve::Linear => (1.0 - t, t),
            FadeCurve::Sinusoidal => {
                // Constant power: cos for out, sin for in
                ((t * HALF_PI).cos(), (t * HALF_PI).sin())
            }
        }
    }
}
