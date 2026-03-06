use serde::{Deserialize, Serialize};

use crate::config::sources::{default_false, default_zero};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct FloweryConfig {
    #[serde(default = "crate::config::sources::default_true")]
    pub enabled: bool,
    #[serde(default = "default_voice")]
    pub voice: String,
    #[serde(default = "default_false")]
    pub translate: bool,
    #[serde(default = "default_zero")]
    pub silence: usize,
    #[serde(default = "default_speed")]
    pub speed: f32,
    #[serde(default = "default_false")]
    pub enforce_config: bool,
}

impl Default for FloweryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            voice: default_voice(),
            translate: false,
            silence: 0,
            speed: default_speed(),
            enforce_config: false,
        }
    }
}

fn default_voice() -> String {
    "Salli".to_string()
}
fn default_speed() -> f32 {
    1.0
}
