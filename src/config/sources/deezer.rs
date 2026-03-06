use serde::{Deserialize, Serialize};

use super::HttpProxyConfig;
use crate::config::sources::{default_false, default_true};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct DeezerConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub arls: Option<Vec<String>>,
    pub master_decryption_key: Option<String>,
    pub proxy: Option<HttpProxyConfig>,
}

impl Default for DeezerConfig {
    fn default() -> Self {
        Self {
            enabled: default_false(),
            arls: None,
            master_decryption_key: None,
            proxy: None,
        }
    }
}
