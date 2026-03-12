use tokio::sync::Mutex;

use crate::{audio::Mixer, common::types::Shared, gateway::constants::DEFAULT_SAMPLE_RATE};

pub struct VoiceEngine {
    pub mixer: Shared<Mixer>,
    pub dave: Option<Shared<crate::gateway::DaveHandler>>,
}

impl VoiceEngine {
    /// Constructs a new VoiceEngine with a shared, mutex-protected Mixer initialized to the default sample rate and no DaveHandler assigned.
    ///
    /// The returned engine contains a Mixer created with `DEFAULT_SAMPLE_RATE`, wrapped in a `Shared<Mutex<...>>`, and an empty `dave` field (`None`).
    ///
    /// # Examples
    ///
    /// ```
    /// let engine = VoiceEngine::new();
    /// assert!(engine.dave.is_none());
    /// ```
    pub fn new() -> Self {
        Self {
            mixer: Shared::new(Mutex::new(Mixer::new(DEFAULT_SAMPLE_RATE))),
            dave: None,
        }
    }
}

impl Default for VoiceEngine {
    fn default() -> Self {
        Self::new()
    }
}
