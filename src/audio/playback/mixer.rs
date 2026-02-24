use std::sync::{
    Arc,
    atomic::{AtomicU8, AtomicU32, AtomicU64, Ordering},
};

use crate::{
    audio::{
        buffer::PooledBuffer,
        playback::{
            effects::{
                TransitionEffect,
                tape::{TapeEffect, TapeState},
            },
            handle::PlaybackState,
        },
    },
    configs::player::PlayerConfig,
};

const MIXER_CHANNELS: u64 = 2;

pub struct Mixer {
    tracks: Vec<MixerTrack>,
    mix_buf: Vec<i32>,
    /// Passthrough channel: raw Opus frames that bypass the PCM mix entirely.
    /// Only set for YouTube WebM/Opus tracks when no filters are active.
    opus_passthrough: Option<PassthroughTrack>,
}

struct PassthroughTrack {
    rx: flume::Receiver<std::sync::Arc<Vec<u8>>>,
    position: Arc<AtomicU64>,
}

struct MixerTrack {
    rx: flume::Receiver<PooledBuffer>,
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
            opus_passthrough: None,
        }
    }

    pub fn add_track(
        &mut self,
        rx: flume::Receiver<PooledBuffer>,
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

    /// Register a raw-Opus passthrough receiver.
    ///
    /// When set, `take_opus_frame()` polls this receiver before the PCM mix
    /// path.  The speak_loop sends the returned frame directly to Discord
    /// without encoding.
    pub fn add_passthrough_track(
        &mut self,
        opus_rx: flume::Receiver<std::sync::Arc<Vec<u8>>>,
        position: Arc<AtomicU64>,
    ) {
        self.opus_passthrough = Some(PassthroughTrack {
            rx: opus_rx,
            position,
        });
    }

    /// Try to receive one raw Opus frame from the passthrough receiver.
    ///
    /// Returns `Some(frame)` if a frame is ready, `None` if not (use PCM path).
    pub fn take_opus_frame(&mut self) -> Option<std::sync::Arc<Vec<u8>>> {
        if let Some(ref pt) = self.opus_passthrough {
            match pt.rx.try_recv() {
                Ok(frame) => {
                    // Update position. Every Opus frame is 20ms (960 samples @ 48kHz).
                    pt.position.fetch_add(960, Ordering::Relaxed);
                    return Some(frame);
                }
                Err(flume::TryRecvError::Disconnected) => {
                    // Processor finished — clear passthrough slot.
                    self.opus_passthrough = None;
                }
                Err(flume::TryRecvError::Empty) => {}
            }
        }
        None
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

                    if i > 0 {
                        track
                            .position
                            .fetch_add(i as u64 / MIXER_CHANNELS, Ordering::Relaxed);
                    }
                }
            } else {
                // ── Normal Playback (Rate 1.0) ────────────────────────────────────
                track.effect = None;
                // ── 1. Drain leftover samples from the previous frame ─────────────
                let pending_len = track.pending.len();
                if track.pending_pos < pending_len {
                    let to_copy = (out_len - i).min(pending_len - track.pending_pos);
                    let vol_fixed = (vol * 65536.0) as i32;

                    if vol_fixed == 65536 {
                        for j in 0..to_copy {
                            self.mix_buf[i + j] += track.pending[track.pending_pos + j] as i32;
                        }
                    } else if vol_fixed != 0 {
                        for j in 0..to_copy {
                            let sample = track.pending[track.pending_pos + j] as i32;
                            self.mix_buf[i + j] += (sample * vol_fixed) >> 16;
                        }
                    }

                    track.pending_pos += to_copy;
                    i += to_copy;
                    track_contributed = true;

                    if track.pending_pos >= pending_len {
                        track.pending.clear();
                        track.pending_pos = 0;
                    }
                }

                // ── 2. receive whole frame batches ────────────────────────────────
                while i < out_len {
                    match track.rx.try_recv() {
                        Ok(frame) => {
                            let frame_len = frame.len();
                            let can_use = frame_len.min(out_len - i);
                            let vol_fixed = (vol * 65536.0) as i32;

                            if vol_fixed == 65536 {
                                for j in 0..can_use {
                                    self.mix_buf[i + j] += frame[j] as i32;
                                }
                            } else if vol_fixed != 0 {
                                // Optimized fixed-point multiplication
                                for j in 0..can_use {
                                    let sample = frame[j] as i32;
                                    self.mix_buf[i + j] += (sample * vol_fixed) >> 16;
                                }
                            }

                            i += can_use;
                            track_contributed = true;

                            if can_use < frame_len {
                                track.pending.extend_from_slice(&frame[can_use..]);
                                track.pending_pos = 0;
                            }
                        }
                        Err(flume::TryRecvError::Disconnected) => {
                            finished = true;
                            break;
                        }
                        Err(flume::TryRecvError::Empty) => break,
                    }
                }

                // Track position using MIXER_CHANNELS (always stereo out to Opus).
                track
                    .position
                    .fetch_add(i as u64 / MIXER_CHANNELS, Ordering::Relaxed);
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
