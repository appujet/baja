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
    #[serde(default)]
    pub applemusic: bool,
    #[serde(default)]
    pub gaana: bool,
    #[serde(default)]
    pub tidal: bool,
    #[serde(default)]
    pub soundcloud: bool,
    #[serde(default)]
    pub audiomack: bool,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct YouTubeConfig {
    #[serde(default)]
    pub clients: YouTubeClientsConfig,
    #[serde(default)]
    pub cipher: YouTubeCipherConfig,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct YouTubeCipherConfig {
    pub url: Option<String>,
    pub token: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct YouTubeClientsConfig {
    #[serde(default = "default_yt_search_clients_list")]
    pub search: Vec<String>,
    #[serde(default = "default_yt_playback_clients_list")]
    pub playback: Vec<String>,
    #[serde(default = "default_yt_resolve_clients_list")]
    pub resolve: Vec<String>,
    #[serde(default)]
    pub refresh_tokens: Vec<String>,
    #[serde(default)]
    pub get_oauth_token: bool,
}

impl Default for YouTubeClientsConfig {
    fn default() -> Self {
        Self {
            search: default_yt_search_clients_list(),
            playback: default_yt_playback_clients_list(),
            resolve: default_yt_resolve_clients_list(),
            refresh_tokens: Vec::new(),
            get_oauth_token: false,
        }
    }
}

fn default_yt_search_clients_list() -> Vec<String> {
    vec![
        "MUSIC_ANDROID".to_string(),
        "MUSIC_WEB".to_string(),
        "ANDROID".to_string(),
        "WEB".to_string(),
    ]
}

fn default_yt_playback_clients_list() -> Vec<String> {
    vec![
        "TV".to_string(),
        "ANDROID_MUSIC".to_string(),
        "WEB".to_string(),
        "IOS".to_string(),
        "ANDROID_VR".to_string(),
    ]
}

fn default_yt_resolve_clients_list() -> Vec<String> {
    vec![
        "WEB".to_string(),
        "MUSIC_WEB".to_string(),
        "ANDROID".to_string(),
        "TVHTML5_SIMPLY".to_string(),
    ]
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

fn default_js_search_limit() -> usize {
    10
}
fn default_js_recommendations_limit() -> usize {
    10
}
fn default_js_playlist_load_limit() -> usize {
    50
}
fn default_js_album_load_limit() -> usize {
    50
}
fn default_js_artist_load_limit() -> usize {
    20
}

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

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct AppleMusicConfig {
    pub country_code: String,
    pub media_api_token: Option<String>,
    #[serde(default = "default_am_playlist_load_limit")]
    pub playlist_load_limit: usize,
    #[serde(default = "default_am_album_load_limit")]
    pub album_load_limit: usize,
    #[serde(default = "default_am_playlist_page_load_concurrency")]
    pub playlist_page_load_concurrency: usize,
    #[serde(default = "default_am_album_page_load_concurrency")]
    pub album_page_load_concurrency: usize,
}

fn default_am_playlist_load_limit() -> usize {
    0
}
fn default_am_album_load_limit() -> usize {
    0
}
fn default_am_playlist_page_load_concurrency() -> usize {
    5
}
fn default_am_album_page_load_concurrency() -> usize {
    5
}

impl Default for AppleMusicConfig {
    fn default() -> Self {
        Self {
            country_code: "us".to_string(),
            media_api_token: None,
            playlist_load_limit: default_am_playlist_load_limit(),
            album_load_limit: default_am_album_load_limit(),
            playlist_page_load_concurrency: default_am_playlist_page_load_concurrency(),
            album_page_load_concurrency: default_am_album_page_load_concurrency(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct MirrorsConfig {
    #[serde(default)]
    pub providers: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GaanaConfig {
    pub proxy: Option<HttpProxyConfig>,
    pub stream_quality: Option<String>,
    #[serde(default = "default_gn_search_limit")]
    pub search_limit: usize,
    #[serde(default = "default_gn_playlist_load_limit")]
    pub playlist_load_limit: usize,
    #[serde(default = "default_gn_album_load_limit")]
    pub album_load_limit: usize,
    #[serde(default = "default_gn_artist_load_limit")]
    pub artist_load_limit: usize,
}

fn default_gn_search_limit() -> usize {
    10
}
fn default_gn_playlist_load_limit() -> usize {
    50
}
fn default_gn_album_load_limit() -> usize {
    50
}
fn default_gn_artist_load_limit() -> usize {
    20
}

impl Default for GaanaConfig {
    fn default() -> Self {
        Self {
            proxy: None,
            stream_quality: None,
            search_limit: default_gn_search_limit(),
            playlist_load_limit: default_gn_playlist_load_limit(),
            album_load_limit: default_gn_album_load_limit(),
            artist_load_limit: default_gn_artist_load_limit(),
        }
    }
}
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct TidalConfig {
    pub country_code: String,
    pub token: Option<String>,
    #[serde(default = "default_td_playlist_load_limit")]
    pub playlist_load_limit: usize,
    #[serde(default = "default_td_album_load_limit")]
    pub album_load_limit: usize,
    #[serde(default = "default_td_artist_load_limit")]
    pub artist_load_limit: usize,
}

fn default_td_playlist_load_limit() -> usize {
    50
}
fn default_td_album_load_limit() -> usize {
    50
}
fn default_td_artist_load_limit() -> usize {
    20
}

impl Default for TidalConfig {
    fn default() -> Self {
        Self {
            country_code: "US".to_string(),
            token: None,
            playlist_load_limit: default_td_playlist_load_limit(),
            album_load_limit: default_td_album_load_limit(),
            artist_load_limit: default_td_artist_load_limit(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct SoundCloudConfig {
    /// Optional manual client_id override (auto-extracted from soundcloud.com if not set)
    pub client_id: Option<String>,
    /// Proxy configuration
    pub proxy: Option<HttpProxyConfig>,
    #[serde(default = "default_sc_search_limit")]
    pub search_limit: usize,
    #[serde(default = "default_sc_playlist_load_limit")]
    pub playlist_load_limit: usize,
}

fn default_sc_search_limit() -> usize {
    10
}
fn default_sc_playlist_load_limit() -> usize {
    100
}

impl Default for SoundCloudConfig {
    fn default() -> Self {
        Self {
            client_id: None,
            proxy: None,
            search_limit: default_sc_search_limit(),
            playlist_load_limit: default_sc_playlist_load_limit(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct AudiomackConfig {
    #[serde(default = "default_amk_search_limit")]
    pub search_limit: usize,
}

fn default_amk_search_limit() -> usize {
    20
}

impl Default for AudiomackConfig {
    fn default() -> Self {
        Self {
            search_limit: default_amk_search_limit(),
        }
    }
}
