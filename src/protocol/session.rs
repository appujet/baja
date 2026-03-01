use serde::{Deserialize, Serialize};

/// Request body for PATCH /v4/sessions/{sessionId}.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionUpdate {
    #[serde(default)]
    pub resuming: Option<bool>,
    #[serde(default)]
    pub timeout: Option<u64>,
}

/// Response from PATCH /v4/sessions/{sessionId}.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub resuming: bool,
    pub timeout: u64,
}
