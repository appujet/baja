use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct FormatId {
    pub itag: i32,
    pub last_modified: Option<i64>,
    pub xtags: Option<String>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct TimeRange {
    pub start_ticks: i64,
    pub duration_ticks: i64,
    pub timescale: i32,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct MediaHeader {
    pub header_id: i32,
    pub itag: i32,
    pub lmt: Option<String>,
    pub xtags: Option<String>,
    pub is_init_seg: bool,
    pub sequence_number: i32,
    pub start_ms: String,
    pub duration_ms: String,
    pub content_length: Option<String>,
    pub format_id: Option<FormatId>,
    pub time_range: Option<TimeRange>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct FormatInitializationMetadata {
    pub format_id: Option<FormatId>,
    pub itag: Option<i32>,
    pub end_segment_number: String,
    pub mime_type: String,
    pub duration_units: String,
    pub duration_timescale: String,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct NextRequestPolicy {
    pub target_audio_readahead_ms: i32,
    pub target_video_readahead_ms: i32,
    pub max_time_since_last_request_ms: i32,
    pub backoff_time_ms: i32,
    pub min_audio_readahead_ms: i32,
    pub min_video_readahead_ms: i32,
    pub playback_cookie: Option<Vec<u8>>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct SabrRedirect {
    pub url: String,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct SabrError {
    pub error_type: String,
    pub code: i32,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ClientAbrState {
    pub last_manual_selected_resolution: i32,
    pub sticky_resolution: i32,
    pub client_viewport_is_flexible: bool,
    pub bandwidth_estimate: i64,
    pub player_time_ms: i64,
    pub visibility: i32,
    pub playback_rate: f32,
    pub time_since_last_action_ms: i64,
    pub enabled_track_types_bitfield: i32,
    pub player_state: i64,
    pub drc_enabled: bool,
    pub audio_track_id: String,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ClientInfo {
    pub client_name: i32,
    pub client_version: String,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct SabrContext {
    pub context_type: i32,
    pub value: Vec<u8>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct SabrContextUpdate {
    pub context_type: i32,
    pub value: Vec<u8>,
    pub send_by_default: bool,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct SabrContextSendingPolicy {
    pub start_policy: Vec<i32>,
    pub stop_policy: Vec<i32>,
    pub discard_policy: Vec<i32>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct StreamProtectionStatus {
    pub status: i32,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct StreamerContext {
    pub client_info: Option<ClientInfo>,
    pub po_token: Option<Vec<u8>>,
    pub playback_cookie: Option<Vec<u8>>,
    pub sabr_contexts: Vec<SabrContext>,
    pub unsent_sabr_contexts: Vec<i32>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct BufferedRange {
    pub format_id: Option<FormatId>,
    pub start_time_ms: i64,
    pub duration_ms: i64,
    pub start_segment_index: i32,
    pub end_segment_index: i32,
    pub time_range: Option<TimeRange>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct VideoPlaybackAbrRequest {
    pub client_abr_state: Option<ClientAbrState>,
    pub selected_format_ids: Vec<FormatId>,
    pub buffered_ranges: Vec<BufferedRange>,
    pub player_time_ms: i64,
    pub video_playback_ustreamer_config: Vec<u8>,
    pub preferred_audio_format_ids: Vec<FormatId>,
    pub preferred_video_format_ids: Vec<FormatId>,
    pub streamer_context: Option<StreamerContext>,
}
