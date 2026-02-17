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

pub mod error;
pub mod info;
pub mod messages;
pub mod player;
pub mod routeplanner;
pub mod session;
pub mod stats;
pub mod track;

pub use error::*;
pub use info::*;
pub use messages::*;
pub use player::*;
pub use session::*;
pub use stats::*;
pub use track::*;
