use serde::{Deserialize, Serialize};
use base64::{Engine as _, engine::general_purpose};
use toml::Value;

use crate::configs::*;

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Config {
    pub server: ServerConfig,
    pub route_planner: RoutePlannerConfig,
    pub sources: SourcesConfig,
    #[serde(default)]
    pub lyrics: LyricsConfig,
    pub logging: Option<LoggingConfig>,
    #[serde(default)]
    pub filters: FiltersConfig,
    #[serde(default)]
    pub player: PlayerConfig,
    #[serde(default)]
    pub jiosaavn: Option<JioSaavnConfig>,
    #[serde(default)]
    pub mirrors: Option<MirrorsConfig>,
    #[serde(default)]
    pub spotify: Option<SpotifyConfig>,
    #[serde(default)]
    pub deezer: Option<DeezerConfig>,
    #[serde(default)]
    pub youtube: Option<YouTubeConfig>,
    #[serde(default)]
    pub applemusic: Option<AppleMusicConfig>,
    #[serde(default)]
    pub gaana: Option<GaanaConfig>,
    #[serde(default)]
    pub tidal: Option<TidalConfig>,
    #[serde(default)]
    pub soundcloud: Option<SoundCloudConfig>,
    #[serde(default)]
    pub audiomack: Option<AudiomackConfig>,
    #[serde(default)]
    pub pandora: Option<PandoraConfig>,
    #[serde(default)]
    pub qobuz: Option<QobuzConfig>,
    #[serde(default)]
    pub anghami: Option<AnghamiConfig>,
    #[serde(default)]
    pub shazam: Option<ShazamConfig>,
    #[serde(default)]
    pub mixcloud: Option<MixcloudConfig>,
    #[serde(default)]
    pub bandcamp: Option<BandcampConfig>,
    #[serde(default)]
    pub audius: Option<AudiusConfig>,
    #[serde(default)]
    pub yandexmusic: Option<YandexMusicConfig>,
    #[serde(default)]
    pub yandex: Option<YandexConfig>,
    #[serde(default)]
    pub google_tts: Option<GoogleTtsConfig>,
    #[serde(default)]
    pub flowery: Option<FloweryConfig>,
    #[serde(default)]
    pub lazypytts: Option<LazyPyTtsConfig>,
    #[serde(default)]
    pub config_server: Option<ConfigServerConfig>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ConfigServerConfig {
    pub url: String,
    pub username: Option<String>,
    pub password: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server: ServerConfig::default(),
            route_planner: RoutePlannerConfig::default(),
            sources: SourcesConfig::default(),
            lyrics: LyricsConfig::default(),
            logging: None,
            filters: FiltersConfig::default(),
            player: PlayerConfig::default(),
            jiosaavn: None,
            mirrors: None,
            spotify: None,
            deezer: None,
            youtube: None,
            applemusic: None,
            gaana: None,
            tidal: None,
            soundcloud: None,
            audiomack: None,
            pandora: None,
            qobuz: None,
            anghami: None,
            shazam: None,
            mixcloud: None,
            bandcamp: None,
            audius: None,
            yandexmusic: None,
            yandex: None,
            google_tts: None,
            flowery: None,
            lazypytts: None,
            config_server: None,
        }
    }
}

use crate::common::types::AnyResult;

impl Config {
    pub async fn load() -> AnyResult<Self> {
        let config_path = if std::path::Path::new("config.toml").exists() {
            "config.toml"
        } else if std::path::Path::new("config.default.toml").exists() {
            "config.default.toml"
        } else {
            return Err("config.toml or config.default.toml not found".into());
        };

        crate::log_println!("Loading configuration from: {}", config_path);

        let config_str = std::fs::read_to_string(config_path)?;
        if config_str.is_empty() {
            return Err(format!("{} is empty", config_path).into());
        }

        let raw_val: Value = toml::from_str(&config_str)?;
        
        if let Some(cs_val) = raw_val.get("config_server") {
            let cs: ConfigServerConfig = cs_val.clone().try_into()?;
                        
            let client = reqwest::Client::new();
            let mut request = client.get(&cs.url);

            if let (Some(u), Some(p)) = (&cs.username, &cs.password) {
                let auth = format!("{}:{}", u, p);
                let encoded = general_purpose::STANDARD.encode(auth);
                request = request.header("Authorization", format!("Basic {}", encoded));
            }

            let response = request.send().await?;
            if !response.status().is_success() {
                return Err(format!("Failed to fetch remote config: status {}", response.status()).into());
            }

            let remote_toml = response.text().await?;

            return Ok(toml::from_str(&remote_toml)?);
        }

        let config: Config = toml::from_str(&config_str)?;
        Ok(config)
    }
}
