use serde::{Deserialize, Serialize};

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
        }
    }
}

use crate::common::types::AnyResult;

impl Config {
    pub fn load() -> AnyResult<Self> {
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

        let config: Config = toml::from_str(&config_str)?;
        Ok(config)
    }
}
