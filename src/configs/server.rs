use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub password: String,
    #[serde(default = "default_player_update_interval")]
    pub player_update_interval: u64,
    #[serde(default = "default_stats_interval")]
    pub stats_interval: u64,
    #[serde(default = "default_websocket_ping_interval")]
    pub websocket_ping_interval: u64,
    #[serde(default = "default_max_event_queue_size")]
    pub max_event_queue_size: usize,
}

fn default_max_event_queue_size() -> usize {
    100
}

fn default_player_update_interval() -> u64 {
    5
}

fn default_stats_interval() -> u64 {
    60
}

fn default_websocket_ping_interval() -> u64 {
    30
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
