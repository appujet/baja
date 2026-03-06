use serde::{Deserialize, Serialize};

use crate::config::sources::default_true;

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GoogleTtsConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_language")]
    pub language: String,
}

impl Default for GoogleTtsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            language: default_language(),
        }
    }
}

fn default_language() -> String {
    "en-US".to_string()
}
