use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct SpotifyConfig {
    #[serde(rename = "spDc")]
    pub sp_dc: Option<String>,
}
