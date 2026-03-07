use std::{ops::Deref, sync::Arc};

use rand::{Rng, distributions::Alphanumeric};
use tokio::sync::{Mutex, RwLock};

/// A thread-safe, mutually exclusive shared component.
pub type Shared<T> = Arc<Mutex<T>>;

/// A thread-safe, read-write shared component.
pub type SharedRw<T> = Arc<RwLock<T>>;

/// A generic boxed error type.
pub type AnyError = Box<dyn std::error::Error + Send + Sync>;

/// A convenient Result alias returning `AnyError`.
pub type AnyResult<T> = std::result::Result<T, AnyError>;

/// Strongly typed identifiers.
macro_rules! define_id {
    ($name:ident, $type:ty) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub $type);

        impl From<$type> for $name {
            fn from(val: $type) -> Self {
                Self(val)
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
    ($name:ident, $type:ty, copy) => {
        #[derive(
            Debug,
            Clone,
            Copy,
            PartialEq,
            Eq,
            PartialOrd,
            Ord,
            Hash,
            serde::Serialize,
            serde::Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub $type);

        impl From<$type> for $name {
            fn from(val: $type) -> Self {
                Self(val)
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

define_id!(GuildId, String);
define_id!(SessionId, String);
define_id!(UserId, u64, copy);
define_id!(ChannelId, u64, copy);

impl Deref for GuildId {
    type Target = str;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Deref for SessionId {
    type Target = str;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl SessionId {
    /// Generates a random 16-character alphanumeric session ID (a-z, 0-9).
    pub fn generate() -> Self {
        let rng = rand::thread_rng();
        let s: String = rng
            .sample_iter(&Alphanumeric)
            .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
            .take(16)
            .map(char::from)
            .collect();
        Self(s)
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
        if url.contains(".m3u8") || url.contains("/playlist") {
            return Self::Aac;
        }

        // Handle YouTube itag hint
        if let Some(itag) = extract_youtube_itag(url) {
            match itag {
                249..=251 => return Self::Webm,
                139..=141 => return Self::Mp4,
                _ => {}
            }
        }

        if url.contains("mime=audio%2Fwebm") || url.contains("mime=audio/webm") {
            return Self::Webm;
        }
        if url.contains("mime=audio%2Fmp4") || url.contains("mime=audio/mp4") {
            return Self::Mp4;
        }

        let from_path = url
            .split('?')
            .next()
            .and_then(|path| std::path::Path::new(path).extension())
            .and_then(|ext| ext.to_str())
            .map(Self::from_ext)
            .unwrap_or(Self::Unknown);

        if from_path != Self::Unknown {
            return from_path;
        }

        // Final fallback: look for extensions anywhere in the URL (Tidal etc sometimes use it)
        if url.contains(".mp4") || url.contains(".m4a") {
            return Self::Mp4;
        }
        if url.contains(".flac") {
            return Self::Flac;
        }
        if url.contains(".mp3") {
            return Self::Mp3;
        }
        if url.contains(".ogg") {
            return Self::Ogg;
        }
        if url.contains(".webm") {
            return Self::Webm;
        }

        Self::Unknown
    }

    /// Returns true if the format can potentially be passed through without re-encoding.
    pub fn is_opus_passthrough(&self) -> bool {
        matches!(self, Self::Webm | Self::Ogg | Self::Opus)
    }
}

fn extract_youtube_itag(url: &str) -> Option<u32> {
    url.split('?').nth(1)?.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        if k == "itag" { v.parse().ok() } else { None }
    })
}
