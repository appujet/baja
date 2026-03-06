use serde::{Deserialize, Serialize};

use super::HttpProxyConfig;
use crate::config::sources::{default_limit_10, default_limit_50, default_true};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct SpotifyConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_limit_6")]
    pub playlist_load_limit: usize,
    #[serde(default = "default_limit_6")]
    pub album_load_limit: usize,
    #[serde(default = "default_limit_10")]
    pub search_limit: usize,
    #[serde(default = "default_limit_10")]
    pub recommendations_limit: usize,
    #[serde(default = "default_limit_10")]
    pub playlist_page_load_concurrency: usize,
    #[serde(default = "default_limit_5")]
    pub album_page_load_concurrency: usize,
    #[serde(default = "default_limit_50")]
    pub track_resolve_concurrency: usize,
    pub proxy: Option<HttpProxyConfig>,
}

impl Default for SpotifyConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            playlist_load_limit: 6,
            album_load_limit: 6,
            search_limit: 10,
            recommendations_limit: 10,
            playlist_page_load_concurrency: 10,
            album_page_load_concurrency: 5,
            track_resolve_concurrency: 50,
            proxy: None,
        }
    }
}

fn default_limit_6() -> usize {
    6
}

fn default_limit_5() -> usize {
    5
}
