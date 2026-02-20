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

/// Common async task handle.
pub type TaskHandle = tokio::task::JoinHandle<()>;
