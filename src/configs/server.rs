use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct ServerConfig {
  pub host: String,
  pub port: u16,
  pub password: String,
  #[serde(default = "default_player_update_interval")]
  pub player_update_interval: u64,
}

fn default_player_update_interval() -> u64 {
  5
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct LoggingConfig {
  pub level: Option<String>,
  pub filters: Option<String>,
  pub file: Option<LogFileConfig>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct LogFileConfig {
  pub path: String,
  pub max_lines: u32,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct RoutePlannerConfig {
  pub enabled: bool,
  pub cidrs: Vec<String>,
  pub excluded_ips: Vec<String>,
}
