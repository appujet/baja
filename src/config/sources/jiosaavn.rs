use serde::{Deserialize, Serialize};

use super::HttpProxyConfig;
use crate::config::sources::{default_limit_10, default_limit_20, default_limit_50, default_true};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct JioSaavnConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(rename = "apiUrl")]
    pub api_url: Option<String>,
    pub decryption: Option<JioSaavnDecryptionConfig>,
    pub proxy: Option<HttpProxyConfig>,
    #[serde(default = "default_limit_10")]
    pub search_limit: usize,
    #[serde(default = "default_limit_10")]
    pub recommendations_limit: usize,
    #[serde(default = "default_limit_50")]
    pub playlist_load_limit: usize,
    #[serde(default = "default_limit_50")]
    pub album_load_limit: usize,
    #[serde(default = "default_limit_20")]
    pub artist_load_limit: usize,
}

impl Default for JioSaavnConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            api_url: None,
            decryption: None,
            proxy: None,
            search_limit: 10,
            recommendations_limit: 10,
            playlist_load_limit: 50,
            album_load_limit: 50,
            artist_load_limit: 20,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct JioSaavnDecryptionConfig {
    #[serde(rename = "secretKey")]
    pub secret_key: Option<String>,
}
