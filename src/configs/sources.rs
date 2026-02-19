use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct SourcesConfig {
    pub youtube: bool,
    pub spotify: bool,
    pub http: bool,
    pub local: bool,
    #[serde(default)]
    pub jiosaavn: bool,
    #[serde(default)]
    pub deezer: bool,
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

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct JioSaavnConfig {
    pub decryption: Option<JioSaavnDecryptionConfig>,
    pub proxy: Option<HttpProxyConfig>,
    #[serde(default = "default_js_search_limit")]
    pub search_limit: usize,
    #[serde(default = "default_js_recommendations_limit")]
    pub recommendations_limit: usize,
    #[serde(default = "default_js_playlist_load_limit")]
    pub playlist_load_limit: usize,
    #[serde(default = "default_js_album_load_limit")]
    pub album_load_limit: usize,
    #[serde(default = "default_js_artist_load_limit")]
    pub artist_load_limit: usize,
}

fn default_js_search_limit() -> usize { 10 }
fn default_js_recommendations_limit() -> usize { 10 }
fn default_js_playlist_load_limit() -> usize { 50 }
fn default_js_album_load_limit() -> usize { 50 }
fn default_js_artist_load_limit() -> usize { 20 }

impl Default for JioSaavnConfig {
    fn default() -> Self {
        Self {
            decryption: None,
            proxy: None,
            search_limit: default_js_search_limit(),
            recommendations_limit: default_js_recommendations_limit(),
            playlist_load_limit: default_js_playlist_load_limit(),
            album_load_limit: default_js_album_load_limit(),
            artist_load_limit: default_js_artist_load_limit(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct JioSaavnDecryptionConfig {
    #[serde(rename = "secretKey")]
    pub secret_key: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct HttpProxyConfig {
    pub url: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
}



#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct DeezerConfig {
    pub arls: Option<Vec<String>>,
    pub master_decryption_key: Option<String>,
    pub proxy: Option<HttpProxyConfig>,
}

impl Default for DeezerConfig {
    fn default() -> Self {
        Self {
            arls: None,
            master_decryption_key: None,
            proxy: None,
        }
    }
}



#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct MirrorsConfig {
    #[serde(default)]
    pub providers: Vec<String>,
}
