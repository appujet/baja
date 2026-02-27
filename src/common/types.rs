use rand::{Rng, distributions::Alphanumeric};
use std::sync::Arc;

use tokio::sync::{Mutex, RwLock};

/// A thread-safe, mutually exclusive shared component.
pub type Shared<T> = Arc<Mutex<T>>;

/// A thread-safe, read-write shared component.
pub type SharedRw<T> = Arc<RwLock<T>>;

/// A generic boxed error type.
pub type AnyError = Box<dyn std::error::Error + Send + Sync>;

/// A convenient Result alias returning `AnyError`.
pub type AnyResult<T> = std::result::Result<T, AnyError>;

/// Strongly typed identifiers (M1).
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct GuildId(pub String);

impl From<String> for GuildId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl std::ops::Deref for GuildId {
    type Target = str;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::fmt::Display for GuildId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct SessionId(pub String);

impl From<String> for SessionId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl std::ops::Deref for SessionId {
    type Target = str;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl SessionId {
    /// Generates a random 20-character alphanumeric session ID (a-z, 0-9).
    pub fn generate() -> Self {
        let rng = rand::thread_rng();
        let s: String = rng
            .sample_iter(&Alphanumeric)
            .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
            .take(20)
            .map(char::from)
            .collect();
        Self(s)
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(transparent)]
pub struct UserId(pub u64);

impl From<u64> for UserId {
    fn from(u: u64) -> Self {
        Self(u)
    }
}

impl std::fmt::Display for UserId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(transparent)]
pub struct ChannelId(pub u64);

impl From<u64> for ChannelId {
    fn from(u: u64) -> Self {
        Self(u)
    }
}

impl std::fmt::Display for ChannelId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Supported audio formats and containers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AudioFormat {
    Aac,
    Opus,
    Webm,
    Mp4,
    Mp3,
    Ogg,
    Flac,
    Wav,
    Unknown,
}

impl AudioFormat {
    pub fn as_ext(&self) -> &'static str {
        match self {
            Self::Aac => "aac",
            Self::Opus => "opus",
            Self::Webm => "webm",
            Self::Mp4 => "mp4",
            Self::Mp3 => "mp3",
            Self::Ogg => "ogg",
            Self::Flac => "flac",
            Self::Wav => "wav",
            Self::Unknown => "",
        }
    }

    pub fn from_ext(ext: &str) -> Self {
        match ext.to_lowercase().as_str() {
            "aac" => Self::Aac,
            "opus" => Self::Opus,
            "webm" => Self::Webm,
            "mp4" | "m4a" => Self::Mp4,
            "mp3" => Self::Mp3,
            "ogg" => Self::Ogg,
            "flac" => Self::Flac,
            "wav" => Self::Wav,
            _ => Self::Unknown,
        }
    }

    /// Detects the audio format from a URL, often using hints like 'itag' or 'mime'.
    pub fn from_url(url: &str) -> Self {
        // HLS hint
        if url.contains(".m3u8") || url.contains("/playlist") {
            return Self::Aac;
        }

        // YouTube itag hint
        let itag: Option<u32> = url.split('?').nth(1).and_then(|qs| {
            qs.split('&').find_map(|kv| {
                let mut parts = kv.splitn(2, '=');
                if parts.next() == Some("itag") {
                    parts.next().and_then(|v| v.parse().ok())
                } else {
                    None
                }
            })
        });

        match itag {
            Some(249) | Some(250) | Some(251) => return Self::Webm,
            Some(139) | Some(140) | Some(141) => return Self::Mp4,
            _ => {}
        }

        // MIME type hint in URL
        if url.contains("mime=audio%2Fwebm") || url.contains("mime=audio/webm") {
            return Self::Webm;
        }
        if url.contains("mime=audio%2Fmp4") || url.contains("mime=audio/mp4") {
            return Self::Mp4;
        }

        // Extension fallback from URL path
        if let Some(ext) = std::path::Path::new(url.split('?').next().unwrap_or(url))
            .extension()
            .and_then(|s| s.to_str())
        {
            return Self::from_ext(ext);
        }

        Self::Unknown
    }

    /// Returns true if the format can potentially be passed through without re-encoding.
    pub fn is_opus_passthrough(&self) -> bool {
        matches!(self, Self::Webm | Self::Ogg | Self::Opus)
    }
}
