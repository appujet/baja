use crate::{audio::Mixer, common::types::Shared, gateway::constants::DEFAULT_SAMPLE_RATE};

pub struct VoiceEngine {
    pub mixer: Shared<Mixer>,
}

impl Default for VoiceEngine {
    fn default() -> Self {
        Self {
            mixer: Shared::new(tokio::sync::Mutex::new(Mixer::new(DEFAULT_SAMPLE_RATE))),
        }
    }
}

impl VoiceEngine {
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }
}
