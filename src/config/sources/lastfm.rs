use serde::{Deserialize, Serialize};

use super::{default_false, default_limit_10};

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
#[serde(default)]
pub struct LastFmConfig {
    #[serde(default = "default_false")]
    pub enabled: bool,
    pub api_key: Option<String>,
    #[serde(default = "default_limit_10")]
    pub search_limit: usize,
}
