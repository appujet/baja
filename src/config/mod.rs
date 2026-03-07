pub mod filters;
pub mod lyrics;
pub mod metrics;
pub mod player;
pub mod server;
pub mod sources;

use std::{fs, path::Path};

pub use filters::*;
pub use lyrics::*;
pub use metrics::*;
pub use player::*;
use serde::Deserialize;
pub use server::*;
pub use sources::*;

use crate::common::types::AnyResult;

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
    #[serde(default)]
    pub metrics: MetricsConfig,
    #[serde(default)]
    pub config_server: Option<ConfigServerConfig>,
}

impl AppConfig {
    pub async fn load() -> AnyResult<Self> {
        let config_path = if Path::new("config.toml").exists() {
            "config.toml"
        } else if Path::new("config.example.toml").exists() {
            "config.example.toml"
        } else {
            return Err("config.toml or config.example.toml not found — please create one from config.example.toml".into());
        };

        crate::log_println!("Loading configuration from: {}", config_path);

        let raw = fs::read_to_string(config_path)?;
        if raw.is_empty() {
            return Err(format!("{} is empty", config_path).into());
        }

        let raw_val: toml::Value = toml::from_str(&raw)?;

        if let Some(cs_val) = raw_val.get("config_server") {
            let cs: ConfigServerConfig = cs_val.clone().try_into()?;

            let client = reqwest::Client::new();
            let mut request = client.get(&cs.url);

            if let (Some(u), Some(p)) = (&cs.username, &cs.password) {
                use base64::{Engine as _, engine::general_purpose};
                let auth = format!("{}:{}", u, p);
                let encoded = general_purpose::STANDARD.encode(auth);
                request = request.header("Authorization", format!("Basic {}", encoded));
            }

            let response = request.send().await?;
            if !response.status().is_success() {
                return Err(format!(
                    "Failed to fetch remote config: status {}",
                    response.status()
                )
                .into());
            }

            let remote_toml = response.text().await?;
            return Ok(toml::from_str(&remote_toml)?);
        }

        let config: Self = toml::from_str(&raw)?;
        Ok(config)
    }
}
