use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct PlayerConfig {
    #[serde(default = "default_stuck_threshold_ms")]
    pub stuck_threshold_ms: u64,
    #[serde(default = "default_buffer_duration_ms")]
    pub buffer_duration_ms: u64,
    #[serde(default = "default_frame_buffer_duration_ms")]
    pub frame_buffer_duration_ms: u64,
    #[serde(default)]
    pub resampling_quality: ResamplingQuality,
    #[serde(default = "default_opus_encoding_quality")]
    pub opus_encoding_quality: u8,
    #[serde(default)]
    pub tape: TapeConfig,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ResamplingQuality {
    Low,
    #[default]
    Medium,
    High,
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
            buffer_duration_ms: default_buffer_duration_ms(),
            frame_buffer_duration_ms: default_frame_buffer_duration_ms(),
            resampling_quality: ResamplingQuality::default(),
            opus_encoding_quality: default_opus_encoding_quality(),
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

fn default_buffer_duration_ms() -> u64 {
    400
}

fn default_frame_buffer_duration_ms() -> u64 {
    5000
}

fn default_opus_encoding_quality() -> u8 {
    10
}

fn default_tape_stop_duration_ms() -> u64 {
    500
}
