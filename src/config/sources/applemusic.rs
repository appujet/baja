use serde::{Deserialize, Serialize};

use super::HttpProxyConfig;
use crate::config::sources::{default_country_code, default_five, default_true, default_zero};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct AppleMusicConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_country_code")]
    pub country_code: String,
    pub media_api_token: Option<String>,
    #[serde(default = "default_zero")]
    pub playlist_load_limit: usize,
    #[serde(default = "default_zero")]
    pub album_load_limit: usize,
    #[serde(default = "default_five")]
    pub playlist_page_load_concurrency: usize,
    #[serde(default = "default_five")]
    pub album_page_load_concurrency: usize,
    pub proxy: Option<HttpProxyConfig>,
}

impl Default for AppleMusicConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            country_code: default_country_code(),
            media_api_token: None,
            playlist_load_limit: 0,
            album_load_limit: 0,
            playlist_page_load_concurrency: 5,
            album_page_load_concurrency: 5,
            proxy: None,
        }
    }
}
