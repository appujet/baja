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
  rx: flume::Receiver<i16>,
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
    rx: flume::Receiver<i16>,
    state: Arc<AtomicU8>,
    volume: Arc<AtomicU32>,
    position: Arc<AtomicU64>,
  ) {
    self.tracks.push(MixerTrack {
      rx,
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

    // Reset buffer
    for s in self.mix_buf.iter_mut() {
      *s = 0;
    }

    // Clean up stopped tracks
    self
      .tracks
      .retain(|t| t.state.load(Ordering::Acquire) != PlaybackState::Stopped as u8);

    let mut has_audio = false;

    for track in self.tracks.iter_mut() {
      let state = track.state.load(Ordering::Acquire);
      if state != PlaybackState::Playing as u8 {
        continue;
      }

      let vol_bits = track.volume.load(Ordering::Acquire);
      let vol = f32::from_bits(vol_bits);

      // Read samples from track
      let mut i = 0;
      let mut finished = false;
      let mut track_contributed = false;

      while i < buf.len() {
        match track.rx.try_recv() {
          Ok(sample) => {
            self.mix_buf[i] += (sample as f32 * vol) as i32;
            i += 1;
            track_contributed = true;
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

      // Update position (stereo: samples / 2 = frames)
      track.position.fetch_add((i / 2) as u64, Ordering::SeqCst);

      if finished {
        track
          .state
          .store(PlaybackState::Stopped as u8, Ordering::Release);
      }
    }

    // Convert back to i16 with saturation
    for (i, &sample) in self.mix_buf.iter().enumerate() {
      buf[i] = sample.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
    }

    has_audio
  }
}
