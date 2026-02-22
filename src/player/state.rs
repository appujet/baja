use serde::{Deserialize, Serialize};

use crate::api::tracks::Track;

/// Full player state as returned by REST endpoints.
pub fn deserialize_track_encoded<'de, D>(deserializer: D) -> Result<Option<TrackEncoded>, D::Error>
where
  D: serde::Deserializer<'de>,
{
  let value: serde_json::Value = serde::Deserialize::deserialize(deserializer)?;
  match value {
    serde_json::Value::Null => Ok(Some(TrackEncoded::Clear)),
    serde_json::Value::String(s) => Ok(Some(TrackEncoded::Set(s))),
    _ => Err(serde::de::Error::custom("expected string or null")),
  }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Player {
  pub guild_id: crate::common::types::GuildId,
  pub track: Option<Track>,
  pub volume: i32,
  pub paused: bool,
  pub state: PlayerState,
  pub voice: VoiceState,
  pub filters: Filters,
}

#[derive(Debug, Serialize)]
pub struct Players {
  pub players: Vec<Player>,
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

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EndTime {
  Clear,    // JSON: null
  Set(u64), // JSON: number
}

impl Default for EndTime {
  fn default() -> Self {
    Self::Clear
  }
}

/// Request body for PATCH /v4/sessions/{sessionId}/players/{guildId}.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerUpdate {
  #[serde(default, deserialize_with = "deserialize_track_encoded")]
  pub encoded_track: Option<TrackEncoded>,
  #[serde(default)]
  pub identifier: Option<String>,
  #[serde(default)]
  pub track: Option<PlayerUpdateTrack>,
  #[serde(default)]
  pub position: Option<u64>,
  #[serde(default)]
  pub end_time: Option<EndTime>,
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
  #[serde(default, deserialize_with = "deserialize_track_encoded")]
  pub encoded: Option<TrackEncoded>,
  /// Track identifier to resolve. Mutually exclusive with `encoded`.
  #[serde(default)]
  pub identifier: Option<String>,
  /// User data to attach to the track.
  #[serde(default)]
  pub user_data: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum TrackEncoded {
  Clear,       // JSON: null
  Set(String), // JSON: string
}

macro_rules! define_filters {
    ($($field:ident : $type:ty => $name:expr),* $(,)?) => {
        /// All audio filters.
        #[derive(Debug, Clone, Default, Serialize, Deserialize)]
        #[serde(rename_all = "camelCase")]
        pub struct Filters {
            $(
                #[serde(skip_serializing_if = "Option::is_none")]
                pub $field: Option<$type>,
            )*
        }

        impl Filters {
            /// Get names of all supported filters in camelCase.
            pub fn names() -> Vec<String> {
                vec![
                    $($name.into()),*
                ]
            }

            /// Merge incoming partial filter update with existing state.
            pub fn merge_from(&mut self, incoming: Filters) {
                $(
                    if incoming.$field.is_some() {
                        self.$field = incoming.$field;
                    }
                )*
            }

            /// Returns true if every filter field is `None`.
            pub fn is_all_none(&self) -> bool {
                $(
                    self.$field.is_none() &&
                )* true
            }
        }
    };
}

define_filters! {
    volume: f32 => "volume",
    equalizer: Vec<EqBand> => "equalizer",
    karaoke: KaraokeFilter => "karaoke",
    timescale: TimescaleFilter => "timescale",
    tremolo: TremoloFilter => "tremolo",
    vibrato: VibratoFilter => "vibrato",
    distortion: DistortionFilter => "distortion",
    rotation: RotationFilter => "rotation",
    channel_mix: ChannelMixFilter => "channelMix",
    low_pass: LowPassFilter => "lowPass",
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
