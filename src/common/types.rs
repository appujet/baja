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

/// Strongly typed identifiers.
pub type GuildId = String;
pub type SessionId = String;
pub type UserId = u64;

/// Supported audio formats/extensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioKind {
  Aac,
  Opus,
  Webm,
  Mp4,
  Mp3,
  Ogg,
  Flac,
  Wav,
}

impl AudioKind {
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
    }
  }

  pub fn from_ext(ext: &str) -> Option<Self> {
    match ext.to_lowercase().as_str() {
      "aac" => Some(Self::Aac),
      "opus" => Some(Self::Opus),
      "webm" => Some(Self::Webm),
      "mp4" | "m4a" => Some(Self::Mp4),
      "mp3" => Some(Self::Mp3),
      "ogg" => Some(Self::Ogg),
      "flac" => Some(Self::Flac),
      "wav" => Some(Self::Wav),
      _ => None,
    }
  }
}
