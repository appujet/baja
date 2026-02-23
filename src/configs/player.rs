use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct PlayerConfig {
    #[serde(default = "default_stuck_threshold_ms")]
    pub stuck_threshold_ms: u64,
    #[serde(default)]
    pub tape_stop: bool,
    #[serde(default = "default_tape_stop_duration_ms")]
    pub tape_stop_duration_ms: u64,
}

impl Default for PlayerConfig {
    fn default() -> Self {
        Self {
            stuck_threshold_ms: default_stuck_threshold_ms(),
            tape_stop: false,
            tape_stop_duration_ms: default_tape_stop_duration_ms(),
        }
    }
}

fn default_stuck_threshold_ms() -> u64 {
    10000
}

fn default_tape_stop_duration_ms() -> u64 {
    500
}
