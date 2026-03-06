use serde::{Deserialize, Serialize};

use super::HttpProxyConfig;
use crate::config::sources::{default_limit_10, default_true};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct BandcampConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_limit_10")]
    pub search_limit: usize,
    pub proxy: Option<HttpProxyConfig>,
}

impl Default for BandcampConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            search_limit: 10,
            proxy: None,
        }
    }
}
