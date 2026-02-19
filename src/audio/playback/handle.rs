use crate::audio::pipeline::decoder::DecoderCommand;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PlaybackState {
    Playing,
    Paused,
    Stopped,
}

#[derive(Clone)]
pub struct TrackHandle {
    state: Arc<Mutex<PlaybackState>>,
    #[allow(dead_code)]
    #[allow(dead_code)]
    volume: Arc<Mutex<f32>>,
    position: Arc<AtomicU64>, // position in samples
    command_tx: flume::Sender<DecoderCommand>,
}

impl TrackHandle {
    pub fn new(
        command_tx: flume::Sender<DecoderCommand>,
    ) -> (
        Self,
        Arc<Mutex<PlaybackState>>,
        Arc<Mutex<f32>>,
        Arc<AtomicU64>,
    ) {
        let state = Arc::new(Mutex::new(PlaybackState::Playing));
        let volume = Arc::new(Mutex::new(1.0));
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

    #[allow(dead_code)]
    pub async fn pause(&self) {
        let mut state = self.state.lock().await;
        *state = PlaybackState::Paused;
    }

    #[allow(dead_code)]
    pub async fn play(&self) {
        let mut state = self.state.lock().await;
        *state = PlaybackState::Playing;
    }

    pub async fn stop(&self) {
        let mut state = self.state.lock().await;
        *state = PlaybackState::Stopped;
    }

    #[allow(dead_code)]
    pub async fn set_volume(&self, vol: f32) {
        let mut volume = self.volume.lock().await;
        *volume = vol;
    }

    pub async fn get_state(&self) -> PlaybackState {
        *self.state.lock().await
    }

    pub fn get_position(&self) -> u64 {
        // Return position in milliseconds. 48000 samples per second.
        let samples = self.position.load(Ordering::Relaxed);
        (samples * 1000) / 48000
    }

    pub async fn seek(&self, position_ms: u64) {
        // Update atomic position immediately
        let samples = (position_ms * 48000) / 1000;
        self.position.store(samples, Ordering::SeqCst);

        // Send seek command
        let _ = self.command_tx.send(DecoderCommand::Seek(position_ms));
    }
}
