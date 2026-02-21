use std::sync::{
  Arc,
  atomic::{AtomicU8, AtomicU32, AtomicU64, Ordering},
};

use crate::audio::processor::DecoderCommand;

#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(u8)]
pub enum PlaybackState {
  Playing = 0,
  Paused = 1,
  Stopped = 2,
}

impl PlaybackState {
  fn from_u8(v: u8) -> Self {
    match v {
      0 => Self::Playing,
      1 => Self::Paused,
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
}

impl TrackHandle {
  pub fn new(
    command_tx: flume::Sender<DecoderCommand>,
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
      },
      state,
      volume,
      position,
    )
  }

  pub fn pause(&self) {
    self
      .state
      .store(PlaybackState::Paused as u8, Ordering::Release);
  }

  pub fn play(&self) {
    self
      .state
      .store(PlaybackState::Playing as u8, Ordering::Release);
  }

  pub fn stop(&self) {
    self
      .state
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
