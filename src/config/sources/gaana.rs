use serde::{Deserialize, Serialize};

use super::HttpProxyConfig;
use crate::config::sources::{default_limit_10, default_limit_20, default_limit_50, default_true};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GaanaConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub proxy: Option<HttpProxyConfig>,
    pub stream_quality: Option<String>,
    #[serde(default = "default_limit_10")]
    pub search_limit: usize,
    #[serde(default = "default_limit_50")]
    pub playlist_load_limit: usize,
    #[serde(default = "default_limit_50")]
    pub album_load_limit: usize,
    #[serde(default = "default_limit_20")]
    pub artist_load_limit: usize,
}

impl Default for GaanaConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            proxy: None,
            stream_quality: None,
            search_limit: 10,
            playlist_load_limit: 50,
            album_load_limit: 50,
            artist_load_limit: 20,
        }
    }
}
