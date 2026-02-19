use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub password: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct LoggingConfig {
    pub level: Option<String>,
    pub filters: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct RoutePlannerConfig {
    pub enabled: bool,
    pub cidrs: Vec<String>,
    pub excluded_ips: Vec<String>,
}
