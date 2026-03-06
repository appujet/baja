use serde::{Deserialize, Serialize};

use super::HttpProxyConfig;
use crate::config::sources::{
    default_limit_10, default_limit_20, default_limit_50, default_limit_100, default_true,
};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct QobuzConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub user_token: Option<String>,
    pub app_id: Option<String>,
    pub app_secret: Option<String>,
    pub proxy: Option<HttpProxyConfig>,
    #[serde(default = "default_limit_10")]
    pub search_limit: usize,
    #[serde(default = "default_limit_100")]
    pub playlist_load_limit: usize,
    #[serde(default = "default_limit_50")]
    pub album_load_limit: usize,
    #[serde(default = "default_limit_20")]
    pub artist_load_limit: usize,
}

impl Default for QobuzConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            user_token: None,
            app_id: None,
            app_secret: None,
            proxy: None,
            search_limit: 10,
            playlist_load_limit: 100,
            album_load_limit: 50,
            artist_load_limit: 20,
        }
    }
}
