//! `audio/mix/mixer.rs` — unified mixing layer.
//!
//! Contains:
//! - [`AudioMixer`]: multi-layer overlay mixer (sound effects / secondary tracks)
//! - [`Mixer`]: main per-track PCM mixer that drives FlowController and feeds Discord

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU8, AtomicU32, AtomicU64, Ordering},
    },
};

use flume::Receiver;

use crate::audio::{
    buffer::PooledBuffer,
    constants::{MAX_LAYERS, MIXER_CHANNELS},
    flow::FlowController,
    playback::handle::PlaybackState,
};

use crate::configs::player::PlayerConfig;

use super::layer::MixLayer;

// ─── AudioMixer ──────────────────────────────────────────────────────────────

/// Overlays multiple named [`MixLayer`]s onto a main PCM stream.
pub struct AudioMixer {
    pub layers: HashMap<String, MixLayer>,
    pub max_layers: usize,
    pub enabled: bool,
    acc_buf: Vec<i32>,
}

impl AudioMixer {
    pub fn new() -> Self {
        Self {
            layers: HashMap::new(),
            max_layers: MAX_LAYERS,
            enabled: true,
            acc_buf: Vec::with_capacity(1920),
        }
    }

    /// Add a named layer. Returns `Err` if the layer cap is reached.
    pub fn add_layer(
        &mut self,
        id: String,
        rx: Receiver<PooledBuffer>,
        volume: f32,
    ) -> Result<(), &'static str> {
        if self.layers.len() >= self.max_layers {
            return Err("Maximum mix layers reached");
        }
        self.layers
            .insert(id.clone(), MixLayer::new(id, rx, volume));
        Ok(())
    }

    pub fn remove_layer(&mut self, id: &str) {
        self.layers.remove(id);
    }

    pub fn set_layer_volume(&mut self, id: &str, volume: f32) {
        if let Some(layer) = self.layers.get_mut(id) {
            layer.volume = volume.clamp(0.0, 1.0);
        }
    }

    /// Mix all active layers into `main_frame`.
    pub fn mix(&mut self, main_frame: &mut [i16]) {
        if !self.enabled || self.layers.is_empty() {
            return;
        }

        let out_len = main_frame.len();
        if self.acc_buf.len() != out_len {
            self.acc_buf.resize(out_len, 0);
        }

        for (i, &s) in main_frame.iter().enumerate() {
            self.acc_buf[i] = s as i32;
        }

        self.layers.retain(|_, layer| {
            layer.fill();
            !layer.is_dead()
        });

        for layer in self.layers.values_mut() {
            layer.accumulate(&mut self.acc_buf);
        }

        for (i, &sum) in self.acc_buf.iter().enumerate() {
            main_frame[i] = sum.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        }
    }
}

// ─── Mixer ───────────────────────────────────────────────────────────────────

/// Main per-track PCM mixer.
///
/// Each track runs a [`FlowController`] inline (pull-mode via `try_pop_frame`)
/// covering reassembly → filters → tape → volume → fade → crossfade.
/// Secondary audio layers are handled by the embedded [`AudioMixer`].
pub struct Mixer {
    tracks: Vec<MixerTrack>,
    mix_buf: Vec<i32>,
    pub audio_mixer: AudioMixer,
    opus_passthrough: Option<PassthroughTrack>,
    final_pcm_buf: Vec<i16>,
}

struct PassthroughTrack {
    rx: flume::Receiver<Arc<Vec<u8>>>,
    position: Arc<AtomicU64>,
    state: Arc<AtomicU8>,
}

struct MixerTrack {
    /// Effects chain: reassembly → filters → tape → volume → fade → crossfade.
    flow: FlowController,
    /// Overflow from the previous mix tick.
    pending: Vec<i16>,
    pending_pos: usize,
    state: Arc<AtomicU8>,
    volume: Arc<AtomicU32>,
    position: Arc<AtomicU64>,
    config: PlayerConfig,
    finished: bool,
    slice_buf: Vec<i16>,
}

impl Mixer {
    pub fn new(_sample_rate: u32) -> Self {
        Self {
            tracks: Vec::new(),
            mix_buf: Vec::with_capacity(1920),
            audio_mixer: AudioMixer::new(),
            opus_passthrough: None,
            final_pcm_buf: Vec::with_capacity(1920),
        }
    }

    pub fn add_track(
        &mut self,
        rx: Receiver<PooledBuffer>,
        state: Arc<AtomicU8>,
        volume: Arc<AtomicU32>,
        position: Arc<AtomicU64>,
        config: PlayerConfig,
        sample_rate: u32,
    ) {
        let vol_raw = f32::from_bits(volume.load(Ordering::Acquire));
        let mut flow = FlowController::for_mixer(rx, sample_rate, MIXER_CHANNELS);
        flow.volume.set_volume(vol_raw);

        self.tracks.push(MixerTrack {
            flow,
            pending: Vec::new(),
            pending_pos: 0,
            state,
            volume,
            position,
            config,
            finished: false,
            slice_buf: Vec::with_capacity(1920),
        });
    }

    pub fn add_passthrough_track(
        &mut self,
        opus_rx: Receiver<Arc<Vec<u8>>>,
        position: Arc<AtomicU64>,
        state: Arc<AtomicU8>,
    ) {
        self.opus_passthrough = Some(PassthroughTrack {
            rx: opus_rx,
            position,
            state,
        });
    }

    pub fn take_opus_frame(&mut self) -> Option<Arc<Vec<u8>>> {
        if let Some(ref pt) = self.opus_passthrough {
            let state = PlaybackState::from_u8(pt.state.load(Ordering::Acquire));
            if matches!(
                state,
                PlaybackState::Paused
                    | PlaybackState::Stopped
                    | PlaybackState::Stopping
                    | PlaybackState::Starting
            ) {
                return None;
            }
            match pt.rx.try_recv() {
                Ok(frame) => {
                    pt.position.fetch_add(960, Ordering::Relaxed);
                    return Some(frame);
                }
                Err(flume::TryRecvError::Disconnected) => {
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
        self.audio_mixer.enabled = false;
    }

    pub fn mix(&mut self, buf: &mut [i16]) -> bool {
        let out_len = buf.len();

        if self.mix_buf.len() != out_len {
            self.mix_buf.resize(out_len, 0);
        }
        self.mix_buf.fill(0);

        self.tracks
            .retain(|t| t.state.load(Ordering::Acquire) != PlaybackState::Stopped as u8);

        let mut has_audio = false;

        for track in self.tracks.iter_mut() {
            let state = PlaybackState::from_u8(track.state.load(Ordering::Acquire));

            if state == PlaybackState::Paused || state == PlaybackState::Stopped {
                continue;
            }

            // Sync volume.
            let vol_f = f32::from_bits(track.volume.load(Ordering::Acquire));
            if (vol_f - track.flow.volume.current_volume()).abs() > 0.001 {
                track.flow.volume.set_volume(vol_f);
            }

            // Tape transition triggers.
            if state == PlaybackState::Stopping && !track.flow.tape.is_ramping() {
                track.flow.tape.tape_to(
                    track.config.tape.tape_stop_duration_ms as f32,
                    "stop",
                    track.config.tape.curve,
                );
            } else if state == PlaybackState::Starting && !track.flow.tape.is_ramping() {
                track.flow.tape.tape_to(
                    track.config.tape.tape_stop_duration_ms as f32,
                    "start",
                    track.config.tape.curve,
                );
            }

            if track.slice_buf.len() != out_len {
                track.slice_buf.resize(out_len, 0);
            }
            track.slice_buf.fill(0);
            let mut filled = 0usize;

            // 1. Drain overflow from previous tick.
            if track.pending_pos < track.pending.len() {
                let n = (out_len - filled).min(track.pending.len() - track.pending_pos);
                track.slice_buf[filled..filled + n]
                    .copy_from_slice(&track.pending[track.pending_pos..track.pending_pos + n]);
                track.pending_pos += n;
                filled += n;
                if track.pending_pos >= track.pending.len() {
                    track.pending.clear();
                    track.pending_pos = 0;
                }
            }

            // 2. Pull processed frames from FlowController.
            while filled < out_len && !track.finished {
                match track.flow.try_pop_frame() {
                    Ok(Some(frame)) => {
                        let can = frame.len().min(out_len - filled);
                        track.slice_buf[filled..filled + can].copy_from_slice(&frame[..can]);
                        if can < frame.len() {
                            track.pending.extend_from_slice(&frame[can..]);
                            track.pending_pos = 0;
                        }
                        filled += can;
                    }
                    Ok(None) => break,
                    Err(()) => {
                        track.finished = true;
                        break;
                    }
                }
            }

            if filled > 0 {
                for j in 0..filled {
                    self.mix_buf[j] += track.slice_buf[j] as i32;
                }
                has_audio = true;
                track
                    .position
                    .fetch_add(filled as u64 / MIXER_CHANNELS as u64, Ordering::Relaxed);
            }

            if track.finished && track.pending.is_empty() && !track.flow.tape.is_active() {
                track
                    .state
                    .store(PlaybackState::Stopped as u8, Ordering::Release);
            }

            // Tape ramp → state transition.
            if track.flow.tape.check_ramp_completed() {
                match state {
                    PlaybackState::Stopping => {
                        track
                            .state
                            .store(PlaybackState::Paused as u8, Ordering::Release);
                    }
                    PlaybackState::Starting => {
                        track
                            .state
                            .store(PlaybackState::Playing as u8, Ordering::Release);
                    }
                    _ => {}
                }
            }
        }

        if self.final_pcm_buf.len() != out_len {
            self.final_pcm_buf.resize(out_len, 0);
        }

        for (i, &s) in self.mix_buf.iter().enumerate() {
            self.final_pcm_buf[i] = s.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        }

        self.audio_mixer.mix(&mut self.final_pcm_buf);
        if !self.audio_mixer.layers.is_empty() {
            has_audio = true;
        }

        buf.copy_from_slice(&self.final_pcm_buf);
        has_audio
    }
}
