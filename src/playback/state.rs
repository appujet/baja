use crate::api::tracks::Track;
use serde::{Deserialize, Serialize};

/// Full player state as returned by REST endpoints.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Player {
    pub guild_id: String,
    pub track: Option<Track>,
    pub volume: i32,
    pub paused: bool,
    pub state: PlayerState,
    pub voice: VoiceState,
    pub filters: Filters,
}

/// Player connection state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerState {
    /// Unix timestamp in milliseconds.
    pub time: u64,
    /// Playback position in milliseconds.
    pub position: u64,
    /// Whether the player is connected to a voice channel.
    pub connected: bool,
    /// Voice gateway ping in milliseconds. -1 if not connected.
    pub ping: i64,
}

/// Voice connection state.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceState {
    pub token: String,
    pub endpoint: String,
    pub session_id: String,
    #[serde(default)]
    pub channel_id: Option<String>,
}

#[derive(Clone, Default)]
pub struct VoiceConnectionState {
    pub token: String,
    pub endpoint: String,
    pub session_id: String,
    pub channel_id: Option<String>,
}

/// Request body for PATCH /v4/sessions/{sessionId}/players/{guildId}.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerUpdate {
    #[serde(default)]
    pub track: Option<PlayerUpdateTrack>,
    #[serde(default)]
    pub position: Option<u64>,
    #[serde(default)]
    pub end_time: Option<Option<u64>>,
    #[serde(default)]
    pub volume: Option<i32>,
    #[serde(default)]
    pub paused: Option<bool>,
    #[serde(default)]
    pub filters: Option<Filters>,
    #[serde(default)]
    pub voice: Option<VoiceState>,
}

/// Track field in a player update request.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerUpdateTrack {
    /// Base64-encoded track. Null to stop. Omit to keep current.
    #[serde(
        default,
        deserialize_with = "crate::api::deserialize_optional_optional"
    )]
    pub encoded: Option<Option<String>>,
    /// Track identifier to resolve. Mutually exclusive with `encoded`.
    #[serde(default)]
    pub identifier: Option<String>,
    /// User data to attach to the track.
    #[serde(default)]
    #[allow(dead_code)]
    pub user_data: Option<serde_json::Value>,
}

/// All audio filters.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Filters {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub equalizer: Option<Vec<EqBand>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub karaoke: Option<KaraokeFilter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timescale: Option<TimescaleFilter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tremolo: Option<TremoloFilter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vibrato: Option<VibratoFilter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distortion: Option<DistortionFilter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotation: Option<RotationFilter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_mix: Option<ChannelMixFilter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub low_pass: Option<LowPassFilter>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EqBand {
    pub band: u8,
    pub gain: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KaraokeFilter {
    pub level: Option<f32>,
    pub mono_level: Option<f32>,
    pub filter_band: Option<f32>,
    pub filter_width: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimescaleFilter {
    pub speed: Option<f64>,
    pub pitch: Option<f64>,
    pub rate: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TremoloFilter {
    pub frequency: Option<f32>,
    pub depth: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VibratoFilter {
    pub frequency: Option<f32>,
    pub depth: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DistortionFilter {
    pub sin_offset: Option<f32>,
    pub sin_scale: Option<f32>,
    pub cos_offset: Option<f32>,
    pub cos_scale: Option<f32>,
    pub tan_offset: Option<f32>,
    pub tan_scale: Option<f32>,
    pub offset: Option<f32>,
    pub scale: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RotationFilter {
    pub rotation_hz: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelMixFilter {
    pub left_to_left: Option<f32>,
    pub left_to_right: Option<f32>,
    pub right_to_left: Option<f32>,
    pub right_to_right: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LowPassFilter {
    pub smoothing: Option<f32>,
}
