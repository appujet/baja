use std::sync::{
    Arc,
    atomic::{AtomicU8, AtomicU64, Ordering},
};

use super::TransitionEffect;
use crate::audio::playback::handle::PlaybackState;

pub struct TapeEffect {
    pub state: TapeState,
    pub rate: f32,
    pub pos: f32,
    pub step: f32,
}

#[derive(PartialEq)]
pub enum TapeState {
    Stopping,
    Starting,
}

impl TapeEffect {
    pub fn new(state: TapeState, duration_ms: u64) -> Self {
        let rate = match state {
            TapeState::Stopping => 1.0,
            TapeState::Starting => 0.0,
        };
        // 48kHz stereo frames per ms = 48.0
        // We want to complete the transition in duration_ms.
        let frames = (duration_ms as f32 * 48.0).max(1.0);
        let step = 1.0 / frames;

        Self {
            state,
            rate,
            pos: 0.0,
            step,
        }
    }
}

impl TransitionEffect for TapeEffect {
    fn process(
        &mut self,
        mix_buf: &mut [i32],
        i: &mut usize,
        out_len: usize,
        vol: f32,
        stash: &mut Vec<i16>,
        rx: &flume::Receiver<crate::audio::buffer::PooledBuffer>,
        state_atomic: &Arc<AtomicU8>,
        position_atomic: &Arc<AtomicU64>,
    ) -> bool {
        let is_stopping = self.state == TapeState::Stopping;
        let is_starting = self.state == TapeState::Starting;
        let step = self.step;
        let mut track_contributed = false;
        let mut samples_consumed = 0.0f32;

        while *i < out_len {
            if is_stopping {
                self.rate = (self.rate - step).max(0.0);
                if self.rate <= 0.0 {
                    state_atomic.store(PlaybackState::Paused as u8, Ordering::Release);
                    break;
                }
            } else if is_starting {
                self.rate = (self.rate + step).min(1.0);
                if self.rate >= 1.0 {
                    state_atomic.store(PlaybackState::Playing as u8, Ordering::Release);
                    // We don't break immediately, we finish this frame for smoothness
                }
            }

            let read_idx = self.pos.floor() as usize;
            let frac = self.pos - read_idx as f32;

            // Ensure we have enough data in stash
            if (read_idx + 1) * 2 + 1 >= stash.len() {
                match rx.try_recv() {
                    Ok(frame) => {
                        stash.extend_from_slice(&frame);
                        // Re-check after refill
                        if (read_idx + 1) * 2 + 1 >= stash.len() {
                            break;
                        }
                    }
                    _ => break,
                }
            }

            // Linear interpolation (L/R)
            for ch in 0..2 {
                let s0 = stash[read_idx * 2 + ch] as f32;
                let s1 = stash[(read_idx + 1) * 2 + ch] as f32;
                let out_s = s0 + (s1 - s0) * frac;
                mix_buf[*i + ch] += (out_s * vol) as i32;
            }

            let prev_pos = self.pos;
            self.pos += self.rate;
            samples_consumed += self.pos - prev_pos;
            *i += 2;
            track_contributed = true;
        }

        position_atomic.fetch_add(samples_consumed as u64, Ordering::Relaxed);

        track_contributed
    }
}
