use thiserror::Error;

#[derive(Debug, Error)]
pub enum TidalError {
    #[error("API request failed: {0}")]
    Request(#[from] reqwest::Error),

    #[error("API returned {status}: {body}")]
    ApiError {
        status: reqwest::StatusCode,
        body: String,
    },

    #[error("Failed to parse response: {0}")]
    Parse(#[from] serde_json::Error),

    #[error("Base64 decode failed: {0}")]
    Base64(#[from] base64::DecodeError),

    #[error("Authentication failed: {0}")]
    Auth(String),

    #[error("No token available")]
    NoToken,

    #[error("No stream URL in manifest")]
    NoStreamUrl,

    #[error("Other error: {0}")]
    Other(String),
}

pub type TidalResult<T> = Result<T, TidalError>;
