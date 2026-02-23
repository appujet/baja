use std::sync::{
    Arc,
    atomic::{AtomicU8, AtomicU32, AtomicU64, Ordering},
};

use crate::{
    audio::playback::{
        effects::{
            TransitionEffect,
            tape::{TapeEffect, TapeState},
        },
        handle::PlaybackState,
    },
    configs::player::PlayerConfig,
};

pub struct Mixer {
    tracks: Vec<MixerTrack>,
    mix_buf: Vec<i32>,
}

struct MixerTrack {
    rx: flume::Receiver<Vec<i16>>,
    /// Partially-consumed frame — leftover samples.
    pending: Vec<i16>,
    pending_pos: usize,
    state: Arc<AtomicU8>,
    volume: Arc<AtomicU32>,
    position: Arc<AtomicU64>,
    config: PlayerConfig,
    /// Active transition effect (tape stop/start, etc.)
    effect: Option<Box<dyn TransitionEffect>>,
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
        config: PlayerConfig,
    ) {
        self.tracks.push(MixerTrack {
            rx,
            pending: Vec::new(),
            pending_pos: 0,
            state,
            volume,
            position,
            config,
            effect: None,
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

        self.mix_buf.fill(0);

        // Clean up stopped tracks
        self.tracks
            .retain(|t| t.state.load(Ordering::Acquire) != PlaybackState::Stopped as u8);

        let mut has_audio = false;

        for track in self.tracks.iter_mut() {
            let state_raw = track.state.load(Ordering::Acquire);
            let state = PlaybackState::from_u8(state_raw);

            if state == PlaybackState::Paused || state == PlaybackState::Stopped {
                track.effect = None;
                continue;
            }

            let vol_bits = track.volume.load(Ordering::Acquire);
            let vol = f32::from_bits(vol_bits);

            let out_len = buf.len();
            let mut i = 0;
            let mut finished = false;
            let mut track_contributed = false;

            // ── Transition Handling ───────────────────────────────────────────
            let is_stopping = state == PlaybackState::Stopping;
            let is_starting = state == PlaybackState::Starting;

            if is_stopping || is_starting {
                // Initialize effect if missing
                if track.effect.is_none() {
                    let tape_state = if is_stopping {
                        TapeState::Stopping
                    } else {
                        TapeState::Starting
                    };
                    track.effect = Some(Box::new(TapeEffect::new(
                        tape_state,
                        track.config.tape_stop_duration_ms,
                    )));
                }

                if let Some(ref mut effect) = track.effect {
                    // Normalize pending buffer (stash)
                    if track.pending_pos > 0 {
                        track.pending.drain(..track.pending_pos);
                        track.pending_pos = 0;
                    }

                    track_contributed = effect.process(
                        &mut self.mix_buf,
                        &mut i,
                        out_len,
                        vol,
                        &mut track.pending,
                        &track.rx,
                        &track.state,
                        &track.position,
                    );
                }
            } else {
                // ── Normal Playback (Rate 1.0) ────────────────────────────────────
                track.effect = None; // Ensure effect is cleared

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

                // ── 2. receive whole frame batches ────────────────────────────────
                while i < out_len {
                    match track.rx.try_recv() {
                        Ok(frame) => {
                            let can_use = frame.len().min(out_len - i);
                            for j in 0..can_use {
                                self.mix_buf[i + j] += (frame[j] as f32 * vol) as i32;
                            }
                            i += can_use;
                            track_contributed = true;

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

                // Track position for normal playback
                track.position.fetch_add((i / 2) as u64, Ordering::Relaxed);
            }

            if track_contributed {
                has_audio = true;
            }

            if finished && track.pending.is_empty() {
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
