pub mod filters;
pub mod lyrics;
pub mod player;
pub mod server;
pub mod sources;

use std::{fs, path::Path};

pub use filters::*;
pub use lyrics::*;
pub use player::*;
use serde::Deserialize;
pub use server::*;
pub use sources::*;

#[derive(Debug, Deserialize, Clone)]
pub struct AppConfig {
    pub server: ServerConfig,
    #[serde(default)]
    pub route_planner: RoutePlannerConfig,
    #[serde(default)]
    pub sources: SourcesConfig,
    #[serde(default)]
    pub lyrics: LyricsConfig,
    pub logging: Option<LoggingConfig>,
    #[serde(default)]
    pub filters: FiltersConfig,
    #[serde(default)]
    pub player: PlayerConfig,
}

impl AppConfig {
    pub fn load() -> Self {
        let config_path = if Path::new("config.toml").exists() {
            "config.toml"
        } else {
            panic!("config.toml not found — please create one from config.example.toml");
        };

        let raw = fs::read_to_string(config_path)
            .unwrap_or_else(|_| panic!("Failed to read {}", config_path));

        toml::from_str(&raw).unwrap_or_else(|err| {
            panic!(
                "{} contains invalid TOML or missing required fields: {}",
                config_path, err
            )
        })
    }
}
