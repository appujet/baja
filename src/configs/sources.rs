use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
#[serde(default)]
pub struct SourcesConfig {
    pub youtube: bool,
    pub spotify: bool,
    pub http: bool,
    pub local: bool,
    pub jiosaavn: bool,
    pub deezer: bool,
    pub applemusic: bool,
    pub gaana: bool,
    pub tidal: bool,
    pub soundcloud: bool,
    pub audiomack: bool,
    pub audius: bool,
    pub pandora: bool,
    pub qobuz: bool,
    pub anghami: bool,
    pub shazam: bool,
    pub mixcloud: bool,
    pub bandcamp: bool,
    pub yandexmusic: bool,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
#[serde(default)]
pub struct YouTubeConfig {
    pub clients: YouTubeClientsConfig,
    pub cipher: YouTubeCipherConfig,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
#[serde(default)]
pub struct YouTubeCipherConfig {
    pub url: Option<String>,
    pub token: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct MixcloudConfig {
    #[serde(default = "default_search_limit")]
    pub search_limit: usize,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct BandcampConfig {
    #[serde(default = "default_search_limit")]
    pub search_limit: usize,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct AudiusConfig {
    #[serde(default = "default_search_limit")]
    pub search_limit: usize,
    #[serde(default = "default_playlist_load_limit")]
    pub playlist_load_limit: usize,
    #[serde(default = "default_album_load_limit")]
    pub album_load_limit: usize,
    #[serde(default)]
    pub app_name: Option<String>,
}

fn default_playlist_load_limit() -> usize {
    100
}

fn default_album_load_limit() -> usize {
    100
}

fn default_search_limit() -> usize {
    10
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct ShazamConfig {
    #[serde(default = "default_search_limit")]
    pub search_limit: usize,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct YouTubeClientsConfig {
    pub search: Vec<String>,
    pub playback: Vec<String>,
    pub resolve: Vec<String>,
    pub refresh_tokens: Vec<String>,
    pub get_oauth_token: bool,
}

impl Default for YouTubeClientsConfig {
    fn default() -> Self {
        Self {
            search: vec![
                "MUSIC_ANDROID".to_string(),
                "MUSIC_WEB".to_string(),
                "ANDROID".to_string(),
                "WEB".to_string(),
            ],
            playback: vec![
                "TV".to_string(),
                "ANDROID_MUSIC".to_string(),
                "WEB".to_string(),
                "IOS".to_string(),
                "ANDROID_VR".to_string(),
            ],
            resolve: vec![
                "WEB".to_string(),
                "MUSIC_WEB".to_string(),
                "ANDROID".to_string(),
                "TVHTML5_SIMPLY".to_string(),
            ],
            refresh_tokens: Vec::new(),
            get_oauth_token: false,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct SpotifyConfig {
    pub playlist_load_limit: usize,
    pub album_load_limit: usize,
    pub search_limit: usize,
    pub recommendations_limit: usize,
    pub playlist_page_load_concurrency: usize,
    pub album_page_load_concurrency: usize,
    pub track_resolve_concurrency: usize,
}

impl Default for SpotifyConfig {
    fn default() -> Self {
        Self {
            playlist_load_limit: 6,
            album_load_limit: 6,
            search_limit: 10,
            recommendations_limit: 10,
            playlist_page_load_concurrency: 10,
            album_page_load_concurrency: 5,
            track_resolve_concurrency: 50,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct JioSaavnConfig {
    pub decryption: Option<JioSaavnDecryptionConfig>,
    pub proxy: Option<HttpProxyConfig>,
    pub search_limit: usize,
    pub recommendations_limit: usize,
    pub playlist_load_limit: usize,
    pub album_load_limit: usize,
    pub artist_load_limit: usize,
}

impl Default for JioSaavnConfig {
    fn default() -> Self {
        Self {
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

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct HttpProxyConfig {
    pub url: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct DeezerConfig {
    pub arls: Option<Vec<String>>,
    pub master_decryption_key: Option<String>,
    pub proxy: Option<HttpProxyConfig>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct AppleMusicConfig {
    pub country_code: String,
    pub media_api_token: Option<String>,
    pub playlist_load_limit: usize,
    pub album_load_limit: usize,
    pub playlist_page_load_concurrency: usize,
    pub album_page_load_concurrency: usize,
}

impl Default for AppleMusicConfig {
    fn default() -> Self {
        Self {
            country_code: "us".to_string(),
            media_api_token: None,
            playlist_load_limit: 0,
            album_load_limit: 0,
            playlist_page_load_concurrency: 5,
            album_page_load_concurrency: 5,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct MirrorsConfig {
    pub providers: Vec<String>,
    #[serde(default = "default_mirrors_timeout")]
    pub timeout_ms: u64,
}

impl Default for MirrorsConfig {
    fn default() -> Self {
        Self {
            providers: Vec::new(),
            timeout_ms: 5000,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct GaanaConfig {
    pub proxy: Option<HttpProxyConfig>,
    pub stream_quality: Option<String>,
    pub search_limit: usize,
    pub playlist_load_limit: usize,
    pub album_load_limit: usize,
    pub artist_load_limit: usize,
}

impl Default for GaanaConfig {
    fn default() -> Self {
        Self {
            proxy: None,
            stream_quality: None,
            search_limit: 10,
            playlist_load_limit: 50,
            album_load_limit: 50,
            artist_load_limit: 20,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct TidalConfig {
    pub country_code: String,
    pub token: Option<String>,
    pub playlist_load_limit: usize,
    pub album_load_limit: usize,
    pub artist_load_limit: usize,
}

impl Default for TidalConfig {
    fn default() -> Self {
        Self {
            country_code: "US".to_string(),
            token: None,
            playlist_load_limit: 50,
            album_load_limit: 50,
            artist_load_limit: 20,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct SoundCloudConfig {
    pub client_id: Option<String>,
    pub proxy: Option<HttpProxyConfig>,
    pub search_limit: usize,
    pub playlist_load_limit: usize,
}

impl Default for SoundCloudConfig {
    fn default() -> Self {
        Self {
            client_id: None,
            proxy: None,
            search_limit: 10,
            playlist_load_limit: 100,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct AudiomackConfig {
    pub search_limit: usize,
}

impl Default for AudiomackConfig {
    fn default() -> Self {
        Self { search_limit: 20 }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct PandoraConfig {
    pub proxy: Option<HttpProxyConfig>,
    pub csrf_token: Option<String>,
    pub search_limit: usize,
    pub playlist_load_limit: usize,
}

impl Default for PandoraConfig {
    fn default() -> Self {
        Self {
            proxy: None,
            csrf_token: None,
            search_limit: 10,
            playlist_load_limit: 100,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct QobuzConfig {
    pub user_token: Option<String>,
    pub app_id: Option<String>,
    pub app_secret: Option<String>,
    pub proxy: Option<HttpProxyConfig>,
    pub search_limit: usize,
    pub playlist_load_limit: usize,
    pub album_load_limit: usize,
    pub artist_load_limit: usize,
}

impl Default for QobuzConfig {
    fn default() -> Self {
        Self {
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

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct AnghamiConfig {
    pub search_limit: usize,
}

impl Default for AnghamiConfig {
    fn default() -> Self {
        Self { search_limit: 10 }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct YandexMusicConfig {
    pub access_token: Option<String>,
    #[serde(default = "default_yandex_playlist_load_limit")]
    pub playlist_load_limit: usize,
    #[serde(default = "default_yandex_album_load_limit")]
    pub album_load_limit: usize,
    #[serde(default = "default_yandex_artist_load_limit")]
    pub artist_load_limit: usize,
    pub proxy: Option<HttpProxyConfig>,
    #[serde(default = "default_search_limit")]
    pub search_limit: usize,
}

fn default_yandex_playlist_load_limit() -> usize {
    6
}

fn default_yandex_album_load_limit() -> usize {
    6
}

fn default_yandex_artist_load_limit() -> usize {
    6
}

fn default_mirrors_timeout() -> u64 {
    5000
}
