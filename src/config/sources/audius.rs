use serde::{Deserialize, Serialize};

use super::HttpProxyConfig;
use crate::config::sources::{default_limit_10, default_limit_100, default_true};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct AudiusConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_limit_10")]
    pub search_limit: usize,
    #[serde(default = "default_limit_100")]
    pub playlist_load_limit: usize,
    #[serde(default = "default_limit_100")]
    pub album_load_limit: usize,
    #[serde(default)]
    pub app_name: Option<String>,
    pub proxy: Option<HttpProxyConfig>,
}

impl Default for AudiusConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            search_limit: 10,
            playlist_load_limit: 100,
            album_load_limit: 100,
            app_name: None,
            proxy: None,
        }
    }
}
