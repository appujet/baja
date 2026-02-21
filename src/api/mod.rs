use serde::{Deserialize, Deserializer};

/// Custom deserializer for `Option<Option<T>>` — distinguishes between:
/// - Field absent → `None`
/// - Field present with `null` → `Some(None)` (e.g., stop the player)
/// - Field present with value → `Some(Some(value))`
pub(crate) fn deserialize_optional_optional<'de, D, T>(
  deserializer: D,
) -> Result<Option<Option<T>>, D::Error>
where
  D: Deserializer<'de>,
  T: Deserialize<'de>,
{
  Ok(Some(Option::deserialize(deserializer)?))
}

pub mod events;
pub mod info;
pub mod models;
pub mod opcodes;
pub mod routeplanner;
pub mod session;
pub mod stats;
pub mod tracks;

pub use events::*;
pub use info::*;
pub use models::*;
pub use routeplanner::*;
pub use session::*;
pub use stats::*;
pub use tracks::*;
