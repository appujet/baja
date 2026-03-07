use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackInfo {
    pub manifest: String,
    pub manifest_mime_type: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub urls: Vec<String>,
    pub mime_type: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceAuthResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: String,
    pub expires_in: u64,
    pub interval: u64,
}

#[derive(Debug, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: u64,
}

#[derive(Clone, Debug)]
pub struct TidalToken {
    pub access_token: String,
    pub expiry_ms: u64,
}

#[derive(Clone, Debug)]
pub enum TidalAuthToken {
    OAuth(String),
    Scraper(String),
}

impl TidalAuthToken {
    pub fn value(&self) -> &str {
        match self {
            Self::OAuth(s) | Self::Scraper(s) => s,
        }
    }
}
