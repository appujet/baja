use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct SourcesConfig {
    pub youtube: bool,
    pub spotify: bool,
    pub http: bool,
    pub local: bool,
    #[serde(default)]
    pub jiosaavn: bool,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct SpotifyConfig {
    #[serde(rename = "spDc")]
    pub sp_dc: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct JioSaavnConfig {
    pub decryption: Option<JioSaavnDecryptionConfig>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct JioSaavnDecryptionConfig {
    #[serde(rename = "secretKey")]
    pub secret_key: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct MirrorsConfig {
    #[serde(default)]
    pub providers: Vec<String>,
}
