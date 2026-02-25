use std::sync::{
    Arc,
    atomic::{AtomicU8, AtomicU32, AtomicU64, Ordering},
};

use crate::{
    audio::{
        buffer::PooledBuffer,
        playback::{
            audio_mixer::AudioMixer,
            effects::{fade::FadeEffect, tape::TapeEffect, volume::VolumeEffect},
            handle::PlaybackState,
        },
    },
    configs::player::PlayerConfig,
};

const MIXER_CHANNELS: usize = 2;

pub struct Mixer {
    tracks: Vec<MixerTrack>,
    mix_buf: Vec<i32>,
    /// Multi-layer mixer for sound effects etc.
    pub audio_mixer: AudioMixer,
    /// Passthrough channel: raw Opus frames that bypass the PCM mix entirely.
    opus_passthrough: Option<PassthroughTrack>,
}

struct PassthroughTrack {
    rx: flume::Receiver<std::sync::Arc<Vec<u8>>>,
    position: Arc<AtomicU64>,
    state: Arc<AtomicU8>,
}

struct MixerTrack {
    rx: flume::Receiver<PooledBuffer>,
    /// The NodeLink-style "Flow" chain for this track.
    /// In a full refactor, this would replace the old transition logic.
    // flow: FlowController,

    /// Old transition components (to be replaced by FlowController ideally,
    /// but keeping for now for minimal breakage).
    pending: Vec<i16>,
    pending_pos: usize,
    state: Arc<AtomicU8>,
    volume: Arc<AtomicU32>,
    position: Arc<AtomicU64>,
    config: PlayerConfig,

    // New components integrated into MixerTrack
    tape: TapeEffect,
    vol_effect: VolumeEffect,
    fade: FadeEffect,
}

impl Mixer {
    pub fn new(_sample_rate: u32) -> Self {
        Self {
            tracks: Vec::new(),
            mix_buf: Vec::with_capacity(1920),
            audio_mixer: AudioMixer::new(),
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
        sample_rate: u32,
    ) {
        let vol_raw = f32::from_bits(volume.load(Ordering::Acquire));
        self.tracks.push(MixerTrack {
            rx,
            pending: Vec::new(),
            pending_pos: 0,
            state,
            volume,
            position,
            config,
            tape: TapeEffect::new(sample_rate, MIXER_CHANNELS),
            vol_effect: VolumeEffect::new(vol_raw, sample_rate, MIXER_CHANNELS),
            fade: FadeEffect::new(1.0, MIXER_CHANNELS),
        });
    }

    pub fn add_passthrough_track(
        &mut self,
        opus_rx: flume::Receiver<std::sync::Arc<Vec<u8>>>,
        position: Arc<AtomicU64>,
        state: Arc<AtomicU8>,
    ) {
        self.opus_passthrough = Some(PassthroughTrack {
            rx: opus_rx,
            position,
            state,
        });
    }

    pub fn take_opus_frame(&mut self) -> Option<std::sync::Arc<Vec<u8>>> {
        if let Some(ref pt) = self.opus_passthrough {
            let state_raw = pt.state.load(Ordering::Acquire);
            let state = PlaybackState::from_u8(state_raw);

            if state == PlaybackState::Paused
                || state == PlaybackState::Stopped
                || state == PlaybackState::Stopping
                || state == PlaybackState::Starting
            {
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
        self.audio_mixer.enabled = false; // or clear it
    }

    pub fn mix(&mut self, buf: &mut [i16]) -> bool {
        if self.mix_buf.len() != buf.len() {
            self.mix_buf.resize(buf.len(), 0);
        }
        self.mix_buf.fill(0);

        self.tracks
            .retain(|t| t.state.load(Ordering::Acquire) != PlaybackState::Stopped as u8);

        let mut has_audio = false;

        for track in self.tracks.iter_mut() {
            let state_raw = track.state.load(Ordering::Acquire);
            let state = PlaybackState::from_u8(state_raw);

            if state == PlaybackState::Paused || state == PlaybackState::Stopped {
                continue;
            }

            let out_len = buf.len();
            let mut finished = false;
            let mut track_contributed = false;

            // Update volume effect from atomic if needed
            let vol_bits = track.volume.load(Ordering::Acquire);
            let vol_f = f32::from_bits(vol_bits);
            if (vol_f - track.vol_effect.current_volume()).abs() > 0.001 {
                track.vol_effect.set_volume(vol_f);
            }

            // NodeLink-style state transition triggers
            if state == PlaybackState::Stopping && !track.tape.is_active() {
                track.tape.tape_to(
                    track.config.tape_stop_duration_ms as f32,
                    "stop",
                    "sinusoidal",
                );
            } else if state == PlaybackState::Starting && !track.tape.is_active() {
                track.tape.tape_to(
                    track.config.tape_stop_duration_ms as f32,
                    "start",
                    "sinusoidal",
                );
            }

            // Main processing slice
            let mut slice = vec![0i16; out_len]; // Temporary frame for effects
            let mut samples_read = 0;

            // 1. Drain pending
            let pending_len = track.pending.len();
            if track.pending_pos < pending_len {
                let to_copy = (out_len - samples_read).min(pending_len - track.pending_pos);
                slice[samples_read..samples_read + to_copy].copy_from_slice(
                    &track.pending[track.pending_pos..track.pending_pos + to_copy],
                );
                track.pending_pos += to_copy;
                samples_read += to_copy;
                if track.pending_pos >= pending_len {
                    track.pending.clear();
                    track.pending_pos = 0;
                }
            }

            // 2. Poll receiver
            while samples_read < out_len {
                match track.rx.try_recv() {
                    Ok(frame) => {
                        let can_use = frame.len().min(out_len - samples_read);
                        slice[samples_read..samples_read + can_use]
                            .copy_from_slice(&frame[..can_use]);
                        if can_use < frame.len() {
                            track.pending.extend_from_slice(&frame[can_use..]);
                            track.pending_pos = 0;
                        }
                        samples_read += can_use;
                    }
                    Err(flume::TryRecvError::Disconnected) => {
                        finished = true;
                        break;
                    }
                    Err(flume::TryRecvError::Empty) => break,
                }
            }

            if samples_read > 0 {
                // Apply NodeLink Effects Chain
                track.tape.process(&mut slice[..samples_read]);
                track.vol_effect.process(&mut slice[..samples_read]);
                track.fade.process(&mut slice[..samples_read]);

                // Accumulate into mix_buf
                for j in 0..samples_read {
                    self.mix_buf[j] += slice[j] as i32;
                }

                track_contributed = true;
                track.position.fetch_add(
                    samples_read as u64 / MIXER_CHANNELS as u64,
                    Ordering::Relaxed,
                );
            }

            if track_contributed {
                has_audio = true;
            }

            if finished && track.pending.is_empty() && !track.tape.is_active() {
                track
                    .state
                    .store(PlaybackState::Stopped as u8, Ordering::Release);
            }

            // If tape ramp finished a 'stop', move to stopped
            if track.tape.check_ramp_completed() && state == PlaybackState::Stopping {
                track
                    .state
                    .store(PlaybackState::Stopped as u8, Ordering::Release);
            }
        }

        // Apply secondary layers (AudioMixer)
        let mut final_pcm = vec![0i16; buf.len()];
        for (i, &s) in self.mix_buf.iter().enumerate() {
            final_pcm[i] = s.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        }

        self.audio_mixer.mix(&mut final_pcm);

        if !self.audio_mixer.layers.is_empty() {
            has_audio = true;
        }

        buf.copy_from_slice(&final_pcm);

        has_audio
    }
}
