use serde::{Deserialize, Serialize};

use super::HttpProxyConfig;
use crate::config::sources::{default_limit_10, default_limit_100, default_true};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct PandoraConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub proxy: Option<HttpProxyConfig>,
    pub csrf_token: Option<String>,
    #[serde(default = "default_limit_10")]
    pub search_limit: usize,
    #[serde(default = "default_limit_100")]
    pub playlist_load_limit: usize,
}

impl Default for PandoraConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            proxy: None,
            csrf_token: None,
            search_limit: 10,
            playlist_load_limit: 100,
        }
    }
}
