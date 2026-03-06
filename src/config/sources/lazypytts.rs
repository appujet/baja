use serde::{Deserialize, Serialize};

use crate::config::sources::{default_false, default_limit_3000, default_true};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct LazyPyTtsConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_service")]
    pub service: String,
    #[serde(default = "default_voice")]
    pub voice: String,
    #[serde(default = "default_false")]
    pub enforce_config: bool,
    #[serde(default = "default_limit_3000")]
    pub max_text_length: usize,
}

impl Default for LazyPyTtsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            service: default_service(),
            voice: default_voice(),
            enforce_config: false,
            max_text_length: 3000,
        }
    }
}

fn default_service() -> String {
    "Cerence".to_string()
}
fn default_voice() -> String {
    "Luciana".to_string()
}
