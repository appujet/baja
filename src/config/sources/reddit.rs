use serde::{Deserialize, Serialize};

use crate::config::sources::HttpProxyConfig;

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
#[serde(default)]
pub struct RedditConfig {
    pub enabled: bool,
    pub proxy: Option<HttpProxyConfig>,
}
