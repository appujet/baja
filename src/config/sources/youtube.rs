use serde::{Deserialize, Serialize};

use crate::config::sources::{default_false, default_true};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct YouTubeConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub clients: YouTubeClientsConfig,
    #[serde(default)]
    pub cipher: YouTubeCipherConfig,
    #[serde(default)]
    pub refresh_tokens: Vec<String>,
    #[serde(default = "default_false")]
    pub get_oauth_token: bool,
}

impl Default for YouTubeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            clients: YouTubeClientsConfig::default(),
            cipher: YouTubeCipherConfig::default(),
            refresh_tokens: Vec::new(),
            get_oauth_token: false,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
#[serde(default)]
pub struct YouTubeCipherConfig {
    pub url: Option<String>,
    pub token: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct YouTubeClientsConfig {
    #[serde(default = "default_search_clients")]
    pub search: Vec<String>,
    #[serde(default = "default_playback_clients")]
    pub playback: Vec<String>,
    #[serde(default = "default_resolve_clients")]
    pub resolve: Vec<String>,
}

impl Default for YouTubeClientsConfig {
    fn default() -> Self {
        Self {
            search: default_search_clients(),
            playback: default_playback_clients(),
            resolve: default_resolve_clients(),
        }
    }
}

fn default_search_clients() -> Vec<String> {
    vec![
        "MUSIC_ANDROID".to_string(),
        "MUSIC_WEB".to_string(),
        "ANDROID".to_string(),
        "WEB".to_string(),
    ]
}

fn default_playback_clients() -> Vec<String> {
    vec![
        "TV".to_string(),
        "ANDROID_MUSIC".to_string(),
        "WEB".to_string(),
        "IOS".to_string(),
        "ANDROID_VR".to_string(),
        "TV_CAST".to_string(),
        "WEB_EMBEDDED".to_string(),
    ]
}

fn default_resolve_clients() -> Vec<String> {
    vec![
        "WEB".to_string(),
        "MUSIC_WEB".to_string(),
        "ANDROID".to_string(),
        "TVHTML5_SIMPLY".to_string(),
    ]
}
