use std::sync::{
    Arc,
    atomic::{AtomicU8, AtomicU32, AtomicU64, Ordering},
};

use crate::audio::playback::handle::PlaybackState;

pub struct Mixer {
    tracks: Vec<MixerTrack>,
    mix_buf: Vec<i32>,
}

struct MixerTrack {
    rx: flume::Receiver<Vec<i16>>,
    /// Partially-consumed frame — leftover samples that didn't fit in the last
    /// mix() call. Stored alongside the read cursor so we don't discard audio.
    pending: Vec<i16>,
    pending_pos: usize,
    state: Arc<AtomicU8>,
    volume: Arc<AtomicU32>,
    position: Arc<AtomicU64>,
}

impl Mixer {
    pub fn new() -> Self {
        Self {
            tracks: Vec::new(),
            mix_buf: Vec::with_capacity(1920),
        }
    }

    pub fn add_track(
        &mut self,
        rx: flume::Receiver<Vec<i16>>,
        state: Arc<AtomicU8>,
        volume: Arc<AtomicU32>,
        position: Arc<AtomicU64>,
    ) {
        self.tracks.push(MixerTrack {
            rx,
            pending: Vec::new(),
            pending_pos: 0,
            state,
            volume,
            position,
        });
    }

    pub fn stop_all(&mut self) {
        for track in self.tracks.iter_mut() {
            track
                .state
                .store(PlaybackState::Stopped as u8, Ordering::Release);
        }
        self.tracks.clear();
    }

    pub fn mix(&mut self, buf: &mut [i16]) -> bool {
        if self.mix_buf.len() != buf.len() {
            self.mix_buf.resize(buf.len(), 0);
        }

        // H3: SIMD-friendly zero-fill (LLVM emits vectorised memset for fill())
        self.mix_buf.fill(0);

        // Clean up stopped tracks
        self.tracks
            .retain(|t| t.state.load(Ordering::Acquire) != PlaybackState::Stopped as u8);

        let mut has_audio = false;

        for track in self.tracks.iter_mut() {
            let state = track.state.load(Ordering::Acquire);
            if state != PlaybackState::Playing as u8 {
                continue;
            }

            let vol_bits = track.volume.load(Ordering::Acquire);
            let vol = f32::from_bits(vol_bits);

            let out_len = buf.len();
            let mut i = 0;
            let mut finished = false;
            let mut track_contributed = false;

            // ── 1. Drain leftover samples from the previous frame ─────────────
            while i < out_len && track.pending_pos < track.pending.len() {
                self.mix_buf[i] += (track.pending[track.pending_pos] as f32 * vol) as i32;
                track.pending_pos += 1;
                i += 1;
                track_contributed = true;
            }
            if track.pending_pos >= track.pending.len() {
                track.pending.clear();
                track.pending_pos = 0;
            }

            // ── 2. H2: receive whole frame batches — one try_recv per packet ───
            // Previously: 1920 try_recv() per frame per track.
            // Now: O(decoded_packets_per_frame) ≈ 2 try_recv() per frame.
            while i < out_len {
                match track.rx.try_recv() {
                    Ok(frame) => {
                        let can_use = frame.len().min(out_len - i);
                        for j in 0..can_use {
                            self.mix_buf[i + j] += (frame[j] as f32 * vol) as i32;
                        }
                        i += can_use;
                        track_contributed = true;

                        // Stash overflow into pending for the next mix() call
                        if can_use < frame.len() {
                            track.pending = frame;
                            track.pending_pos = can_use;
                        }
                    }
                    Err(flume::TryRecvError::Disconnected) => {
                        finished = true;
                        break;
                    }
                    Err(flume::TryRecvError::Empty) => break,
                }
            }

            if track_contributed {
                has_audio = true;
            }

            // H4: position is read only for UI display — Relaxed is sufficient
            track.position.fetch_add((i / 2) as u64, Ordering::Relaxed);

            if finished {
                track
                    .state
                    .store(PlaybackState::Stopped as u8, Ordering::Release);
            }
        }

        // Convert i32 accumulator back to i16 with saturation clamp
        for (i, &sample) in self.mix_buf.iter().enumerate() {
            buf[i] = sample.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        }

        has_audio
    }
}
