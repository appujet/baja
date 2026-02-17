use crate::audio::playback::handle::PlaybackState;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::Mutex;

pub struct Mixer {
    tracks: Vec<MixerTrack>,
    mix_buf: Vec<i32>,
}

struct MixerTrack {
    rx: flume::Receiver<i16>,
    state: Arc<Mutex<PlaybackState>>,
    volume: Arc<Mutex<f32>>,
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
        state: Arc<Mutex<PlaybackState>>,
        volume: Arc<Mutex<f32>>,
        position: Arc<AtomicU64>,
    ) {
        self.tracks.push(MixerTrack {
            rx,
            state,
            volume,
            position,
        });
    }

    pub async fn mix(&mut self, buf: &mut [i16]) -> bool {
        if self.mix_buf.len() != buf.len() {
            self.mix_buf.resize(buf.len(), 0);
        }

        // Reset buffer
        for s in self.mix_buf.iter_mut() {
            *s = 0;
        }

        // Clean up stopped tracks
        self.tracks.retain(|t| {
            if let Ok(state) = t.state.try_lock() {
                *state != PlaybackState::Stopped
            } else {
                true
            }
        });

        let mut has_audio = false;

        for track in self.tracks.iter_mut() {
            let mut state_lock = track.state.lock().await;
            if *state_lock != PlaybackState::Playing {
                continue;
            }

            let vol = *track.volume.lock().await;

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
            track.position.fetch_add((i / 2) as u64, Ordering::Relaxed);

            if finished {
                *state_lock = PlaybackState::Stopped;
            }
        }

        // Convert back to i16 with saturation
        for (i, &sample) in self.mix_buf.iter().enumerate() {
            buf[i] = sample.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        }

        has_audio
    }
}
