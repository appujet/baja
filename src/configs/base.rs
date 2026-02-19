use crate::configs::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Config {
    pub server: ServerConfig,
    pub route_planner: RoutePlannerConfig,
    pub sources: SourcesConfig,
    pub logging: Option<LoggingConfig>,
    #[serde(default)]
    pub filters: FiltersConfig,
    #[serde(default)]
    pub jiosaavn: Option<JioSaavnConfig>,
    #[serde(default)]
    pub mirrors: Option<MirrorsConfig>,
    #[serde(default)]
    pub spotify: Option<SpotifyConfig>,
    #[serde(default)]
    pub youtube: Option<YouTubeConfig>,
}

impl Config {
    pub fn load() -> Result<Self, Box<dyn std::error::Error>> {
        let config_str = std::fs::read_to_string("config.toml").unwrap_or_else(|_| "".to_string());
        if config_str.is_empty() {
            return Err("config.toml not found or empty".into());
        }
        let config: Config = toml::from_str(&config_str)?;
        Ok(config)
    }
}
