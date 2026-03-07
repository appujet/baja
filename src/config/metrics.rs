use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct MetricsConfig {
    #[serde(default)]
    pub prometheus: PrometheusConfig,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct PrometheusConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_prometheus_endpoint")]
    pub endpoint: String,
}

impl Default for PrometheusConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: default_prometheus_endpoint(),
        }
    }
}

fn default_prometheus_endpoint() -> String {
    "/metrics".to_string()
}
