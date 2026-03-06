use serde::{Deserialize, Serialize};

use crate::config::sources::default_true;

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct LocalSourceConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl Default for LocalSourceConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}
