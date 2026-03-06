use serde::{Deserialize, Serialize};

use super::HttpProxyConfig;
use crate::config::sources::{default_limit_10, default_limit_100, default_true};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct SoundCloudConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub client_id: Option<String>,
    pub proxy: Option<HttpProxyConfig>,
    #[serde(default = "default_limit_10")]
    pub search_limit: usize,
    #[serde(default = "default_limit_100")]
    pub playlist_load_limit: usize,
}

impl Default for SoundCloudConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            client_id: None,
            proxy: None,
            search_limit: 10,
            playlist_load_limit: 100,
        }
    }
}
