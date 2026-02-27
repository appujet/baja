use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct PlayerConfig {
    #[serde(default = "default_stuck_threshold_ms")]
    pub stuck_threshold_ms: u64,
    #[serde(default)]
    pub tape: TapeConfig,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct TapeConfig {
    #[serde(default)]
    pub tape_stop: bool,
    #[serde(default = "default_tape_stop_duration_ms")]
    pub tape_stop_duration_ms: u64,
    #[serde(default)]
    pub curve: TapeCurve,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TapeCurve {
    Linear,
    Exponential,
    #[default]
    Sinusoidal,
}

impl TapeCurve {
    pub fn value(self, t: f32) -> f32 {
        match self {
            Self::Linear => t,
            Self::Exponential => t * t,
            Self::Sinusoidal => 0.5 * (1.0 - (t * std::f32::consts::PI).cos()),
        }
    }
}

impl Default for PlayerConfig {
    fn default() -> Self {
        Self {
            stuck_threshold_ms: default_stuck_threshold_ms(),
            tape: TapeConfig::default(),
        }
    }
}

impl Default for TapeConfig {
    fn default() -> Self {
        Self {
            tape_stop: false,
            tape_stop_duration_ms: default_tape_stop_duration_ms(),
            curve: TapeCurve::default(),
        }
    }
}

fn default_stuck_threshold_ms() -> u64 {
    10000
}

fn default_tape_stop_duration_ms() -> u64 {
    500
}
