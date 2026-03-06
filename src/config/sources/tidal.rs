use serde::{Deserialize, Serialize};

use super::HttpProxyConfig;
use crate::config::sources::{
    default_country_code, default_limit_20, default_limit_50, default_true,
};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct TidalConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_country_code")]
    pub country_code: String,
    pub token: Option<String>,
    #[serde(default = "default_limit_50")]
    pub playlist_load_limit: usize,
    #[serde(default = "default_limit_50")]
    pub album_load_limit: usize,
    #[serde(default = "default_limit_20")]
    pub artist_load_limit: usize,
    pub proxy: Option<HttpProxyConfig>,
}

impl Default for TidalConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            country_code: default_country_code(),
            token: None,
            playlist_load_limit: 50,
            album_load_limit: 50,
            artist_load_limit: 20,
            proxy: None,
        }
    }
}
