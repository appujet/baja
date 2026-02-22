use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PlayerConfig {
  #[serde(default = "default_stuck_threshold_ms")]
  pub stuck_threshold_ms: u64,
}

impl Default for PlayerConfig {
  fn default() -> Self {
    Self {
      stuck_threshold_ms: default_stuck_threshold_ms(),
    }
  }
}

fn default_stuck_threshold_ms() -> u64 {
  10000
}
