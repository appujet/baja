
use crate::common::types::{Shared};
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::audio::playback::Mixer;

pub struct VoiceEngine {
    pub mixer: Shared<Mixer>,
}

impl VoiceEngine {
    pub fn new() -> Self {
        Self {
            mixer: Arc::new(Mutex::new(Mixer::new())),
        }
    }
}
