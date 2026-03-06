use serde::{Deserialize, Serialize};

use super::HttpProxyConfig;
use crate::config::sources::{default_limit_20, default_true};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct AudiomackConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_limit_20")]
    pub search_limit: usize,
    pub proxy: Option<HttpProxyConfig>,
}

impl Default for AudiomackConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            search_limit: 20,
            proxy: None,
        }
    }
}
