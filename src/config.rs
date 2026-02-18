use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Config {
    pub server: ServerConfig,
    pub route_planner: RoutePlannerConfig,
    #[serde(default)]
    pub sources: SourcesConfig,
    pub logging: Option<LoggingConfig>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub password: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct RoutePlannerConfig {
    pub enabled: bool,
    pub cidrs: Vec<String>,
    pub excluded_ips: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct SourcesConfig {
    pub youtube: bool,
    pub spotify: bool,
    pub http: bool,
    pub local: bool,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct LoggingConfig {
    pub level: Option<String>,
    pub filters: Option<String>,
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
