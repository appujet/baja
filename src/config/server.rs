use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ServerConfig {
    #[serde(default = "default_address")]
    pub address: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_authorization")]
    pub authorization: String,
    #[serde(default = "default_player_update_interval")]
    pub player_update_interval: u64,
    #[serde(default = "default_stats_interval")]
    pub stats_interval: u64,
    #[serde(default = "default_websocket_ping_interval")]
    pub websocket_ping_interval: u64,
    #[serde(default = "default_max_event_queue_size")]
    pub max_event_queue_size: usize,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            address: default_address(),
            port: default_port(),
            authorization: default_authorization(),
            player_update_interval: default_player_update_interval(),
            stats_interval: default_stats_interval(),
            websocket_ping_interval: default_websocket_ping_interval(),
            max_event_queue_size: default_max_event_queue_size(),
        }
    }
}

fn default_address() -> String {
    "127.0.0.1".to_string()
}
fn default_port() -> u16 {
    2333
}
fn default_authorization() -> String {
    "youshallnotpass".to_string()
}
fn default_max_event_queue_size() -> usize {
    100
}
fn default_player_update_interval() -> u64 {
    5
}
fn default_stats_interval() -> u64 {
    30
}
fn default_websocket_ping_interval() -> u64 {
    20
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct LoggingConfig {
    pub level: Option<String>,
    pub filters: Option<String>,
    pub file: Option<LogFileConfig>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct LogFileConfig {
    pub path: String,
    pub max_lines: u32,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct RoutePlannerConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub cidrs: Vec<String>,
    #[serde(default)]
    pub excluded_ips: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
#[serde(default)]
pub struct MirrorsConfig {
    pub providers: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct ConfigServerConfig {
    pub url: String,
    pub username: Option<String>,
    pub password: Option<String>,
}
