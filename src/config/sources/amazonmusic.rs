use serde::{Deserialize, Serialize};

use crate::config::sources::HttpProxyConfig;

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
#[serde(default)]
pub struct AmazonMusicConfig {
    pub enabled: bool,
    #[serde(default = "default_search_limit")]
    pub search_limit: usize,
    pub proxy: Option<HttpProxyConfig>,
}

fn default_search_limit() -> usize {
    3
}
