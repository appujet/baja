use crate::audio::playback::handle::PlaybackState;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::Mutex;

pub struct Mixer {
    tracks: Vec<MixerTrack>,
}

struct MixerTrack {
    rx: flume::Receiver<f32>,
    state: Arc<Mutex<PlaybackState>>,
    volume: Arc<Mutex<f32>>,
    position: Arc<AtomicU64>,
}

impl Mixer {
    pub fn new() -> Self {
        Self { tracks: Vec::new() }
    }

    pub fn add_track(
        &mut self,
        rx: flume::Receiver<f32>,
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

    pub async fn mix(&mut self, buf: &mut [f32]) {
        // Clear buffer
        for s in buf.iter_mut() {
            *s = 0.0;
        }

        // Clean up stopped tracks
        self.tracks.retain(|_| true);

        for track in self.tracks.iter_mut() {
            let mut state_lock = track.state.lock().await;
            if *state_lock != PlaybackState::Playing {
                continue;
            }

            let vol = *track.volume.lock().await;

            // Read samples from track
            let mut i = 0;
            let mut finished = false;
            while i < buf.len() {
                match track.rx.try_recv() {
                    Ok(sample) => {
                        buf[i] += sample * vol;
                        i += 1;
                    }
                    Err(flume::TryRecvError::Disconnected) => {
                        finished = true;
                        break;
                    }
                    Err(flume::TryRecvError::Empty) => break,
                }
            }

            // Update position (stereo: samples / 2 = frames)
            track.position.fetch_add((i / 2) as u64, Ordering::Relaxed);

            if finished {
                *state_lock = PlaybackState::Stopped;
            }
        }

        // Apply basic limiting/Soft clipping
        for s in buf.iter_mut() {
            if *s > 1.0 {
                *s = 1.0;
            }
            if *s < -1.0 {
                *s = -1.0;
            }
        }
    }
}
