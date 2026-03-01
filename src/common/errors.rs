use serde::{Deserialize, Serialize};

/// Exception severity levels.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Severity {
    Common,
    Suspicious,
    Fault,
}

/// Rustalink v4 JSON error response format.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RustalinkError {
    /// Unix timestamp in milliseconds.
    pub timestamp: u64,
    /// HTTP status code.
    pub status: u16,
    /// HTTP status reason phrase (e.g. "Bad Request").
    pub error: String,
    /// Human-readable error message.
    pub message: String,
    /// The request path that caused the error.
    pub path: String,
    /// Stack trace (only in non-production).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace: Option<String>,
}

impl RustalinkError {
    #[allow(dead_code)]
    pub fn bad_request(message: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            status: 400,
            error: "Bad Request".into(),
            message: message.into(),
            path: path.into(),
            trace: None,
        }
    }

    pub fn not_found(message: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            status: 404,
            error: "Not Found".into(),
            message: message.into(),
            path: path.into(),
            trace: None,
        }
    }

    pub fn internal(message: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            status: 500,
            error: "Internal Server Error".into(),
            message: message.into(),
            path: path.into(),
            trace: None,
        }
    }
}
