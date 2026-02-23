use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering},
};

use crate::audio::processor::DecoderCommand;

#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(u8)]
pub enum PlaybackState {
    Playing = 0,
    Paused = 1,
    Stopped = 2,
    Stopping = 3,
    Starting = 4,
}

impl PlaybackState {
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Playing,
            1 => Self::Paused,
            3 => Self::Stopping,
            4 => Self::Starting,
            _ => Self::Stopped,
        }
    }
}

#[derive(Clone)]
pub struct TrackHandle {
    state: Arc<AtomicU8>,
    volume: Arc<AtomicU32>,   // f32 bits
    position: Arc<AtomicU64>, // position in samples
    command_tx: flume::Sender<DecoderCommand>,
    tape_stop_enabled: Arc<AtomicBool>,
}

impl TrackHandle {
    pub fn new(
        command_tx: flume::Sender<DecoderCommand>,
        tape_stop_enabled: Arc<AtomicBool>,
    ) -> (Self, Arc<AtomicU8>, Arc<AtomicU32>, Arc<AtomicU64>) {
        let state = Arc::new(AtomicU8::new(PlaybackState::Playing as u8));
        let volume = Arc::new(AtomicU32::new(1.0f32.to_bits()));
        let position = Arc::new(AtomicU64::new(0));

        (
            Self {
                state: state.clone(),
                volume: volume.clone(),
                position: position.clone(),
                command_tx,
                tape_stop_enabled,
            },
            state,
            volume,
            position,
        )
    }

    pub fn pause(&self) {
        let next_state = if self.tape_stop_enabled.load(Ordering::Acquire) {
            PlaybackState::Stopping
        } else {
            PlaybackState::Paused
        };
        self.state.store(next_state as u8, Ordering::Release);
    }

    pub fn play(&self) {
        let next_state = if self.tape_stop_enabled.load(Ordering::Acquire) {
            PlaybackState::Starting
        } else {
            PlaybackState::Playing
        };
        self.state.store(next_state as u8, Ordering::Release);
    }

    pub fn stop(&self) {
        self.state
            .store(PlaybackState::Stopped as u8, Ordering::Release);
    }

    pub fn set_volume(&self, vol: f32) {
        self.volume.store(vol.to_bits(), Ordering::Release);
    }

    pub fn get_state(&self) -> PlaybackState {
        let s = self.state.load(Ordering::Acquire);
        let mut state = PlaybackState::from_u8(s);

        if state != PlaybackState::Stopped && self.command_tx.is_disconnected() {
            state = PlaybackState::Stopped;
            self.state.store(state as u8, Ordering::Release);
        }
        state
    }

    pub fn get_position(&self) -> u64 {
        let samples = self.position.load(Ordering::Acquire);
        (samples * 1000) / 48000
    }

    pub fn seek(&self, position_ms: u64) {
        let samples = (position_ms * 48000) / 1000;
        self.position.store(samples, Ordering::Release);
        let _ = self.command_tx.send(DecoderCommand::Seek(position_ms));
    }
}
