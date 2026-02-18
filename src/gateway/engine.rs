use crate::audio::playback::Mixer;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct VoiceEngine {
    pub mixer: Arc<Mutex<Mixer>>,
}

impl VoiceEngine {
    pub fn new() -> Self {
        Self {
            mixer: Arc::new(Mutex::new(Mixer::new())),
        }
    }
}
