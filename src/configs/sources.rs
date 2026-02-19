use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct SourcesConfig {
    pub youtube: bool,
    pub spotify: bool,
    pub http: bool,
    pub local: bool,
    #[serde(default)]
    pub jiosaavn: bool,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct SpotifyConfig {
    #[serde(default = "default_playlist_load_limit")]
    pub playlist_load_limit: usize,
    #[serde(default = "default_album_load_limit")]
    pub album_load_limit: usize,
    #[serde(default = "default_search_limit")]
    pub search_limit: usize,
    #[serde(default = "default_recommendations_limit")]
    pub recommendations_limit: usize,
    #[serde(default = "default_playlist_page_load_concurrency")]
    pub playlist_page_load_concurrency: usize,
    #[serde(default = "default_album_page_load_concurrency")]
    pub album_page_load_concurrency: usize,
    #[serde(default = "default_track_resolve_concurrency")]
    pub track_resolve_concurrency: usize,
}

fn default_playlist_load_limit() -> usize {
    6
}

fn default_album_load_limit() -> usize {
    6
}

fn default_search_limit() -> usize {
    10
}

fn default_recommendations_limit() -> usize {
    10
}

fn default_playlist_page_load_concurrency() -> usize {
    10
}

fn default_album_page_load_concurrency() -> usize {
    5
}

fn default_track_resolve_concurrency() -> usize {
    50
}

impl Default for SpotifyConfig {
    fn default() -> Self {
        Self {
            playlist_load_limit: default_playlist_load_limit(),
            album_load_limit: default_album_load_limit(),
            search_limit: default_search_limit(),
            recommendations_limit: default_recommendations_limit(),
            playlist_page_load_concurrency: default_playlist_page_load_concurrency(),
            album_page_load_concurrency: default_album_page_load_concurrency(),
            track_resolve_concurrency: default_track_resolve_concurrency(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct JioSaavnConfig {
    pub decryption: Option<JioSaavnDecryptionConfig>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct JioSaavnDecryptionConfig {
    #[serde(rename = "secretKey")]
    pub secret_key: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct MirrorsConfig {
    #[serde(default)]
    pub providers: Vec<String>,
}
