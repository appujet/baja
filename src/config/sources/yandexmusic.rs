use serde::{Deserialize, Serialize};

use super::HttpProxyConfig;
use crate::config::sources::{default_false, default_limit_10, default_true};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct YandexMusicConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub access_token: Option<String>,
    #[serde(default = "default_limit_6")]
    pub playlist_load_limit: usize,
    #[serde(default = "default_limit_6")]
    pub album_load_limit: usize,
    #[serde(default = "default_limit_6")]
    pub artist_load_limit: usize,
    pub proxy: Option<HttpProxyConfig>,
    #[serde(default = "default_limit_10")]
    pub search_limit: usize,
}

impl Default for YandexMusicConfig {
    fn default() -> Self {
        Self {
            enabled: default_false(),
            access_token: None,
            playlist_load_limit: 6,
            album_load_limit: 6,
            artist_load_limit: 6,
            proxy: None,
            search_limit: 10,
        }
    }
}

fn default_limit_6() -> usize {
    6
}
