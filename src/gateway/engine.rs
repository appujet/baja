use std::sync::Arc;

use tokio::sync::Mutex;

use crate::{audio::playback::Mixer, common::types::Shared};

pub struct VoiceEngine {
    pub mixer: Shared<Mixer>,
}

impl VoiceEngine {
    pub fn new() -> Self {
        Self {
            mixer: Arc::new(Mutex::new(Mixer::new(48000))),
        }
    }
}
