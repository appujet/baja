//! Lavalink v4 API compatible types.
//!
//! This module defines all the types needed for wire-compatibility with
//! Lavalink v4 clients. Types follow the official Lavalink v4 protocol spec.

use serde::{Deserialize, Deserializer, Serialize};

/// Custom deserializer for `Option<Option<T>>` — distinguishes between:
/// - Field absent → `None`
/// - Field present with `null` → `Some(None)` (e.g., stop the player)
/// - Field present with value → `Some(Some(value))`
fn deserialize_optional_optional<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Some(Option::deserialize(deserializer)?))
}

// ─── Track Types ────────────────────────────────────────────────────────────

/// A single audio track with encoded data and metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    /// Base64-encoded track data.
    pub encoded: String,
    /// Track metadata.
    pub info: TrackInfo,
    /// Plugin-specific info. Always `{}` without plugins.
    #[serde(default)]
    pub plugin_info: serde_json::Value,
    /// User-provided data attached to the track.
    #[serde(default)]
    pub user_data: serde_json::Value,
}

/// Metadata for an audio track.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackInfo {
    pub identifier: String,
    pub is_seekable: bool,
    pub author: String,
    /// Duration in milliseconds. 0 for streams.
    pub length: u64,
    pub is_stream: bool,
    /// Current playback position in milliseconds.
    pub position: u64,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artwork_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub isrc: Option<String>,
    pub source_name: String,
}

// ─── Load Result ────────────────────────────────────────────────────────────

/// Result of a track load operation.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "loadType", content = "data", rename_all = "camelCase")]
pub enum LoadResult {
    /// A single track was loaded.
    Track(Track),
    /// A playlist was loaded.
    Playlist(PlaylistData),
    /// A search returned results.
    Search(Vec<Track>),
    /// No matches found.
    Empty {},
    /// An error occurred during loading.
    Error(LoadError),
}

/// Playlist data returned from a load operation.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistData {
    pub info: PlaylistInfo,
    pub plugin_info: serde_json::Value,
    pub tracks: Vec<Track>,
}

/// Playlist metadata.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistInfo {
    pub name: String,
    /// Index of the selected track, or -1 if none.
    pub selected_track: i32,
}

/// Error from a failed track load.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadError {
    pub message: String,
    pub severity: Severity,
    pub cause: String,
}

/// Exception severity levels.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Severity {
    Common,
    Suspicious,
    Fault,
}

// ─── Player Types ───────────────────────────────────────────────────────────

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

// ─── Player Update (PATCH) ──────────────────────────────────────────────────

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
    #[serde(default, deserialize_with = "deserialize_optional_optional")]
    pub encoded: Option<Option<String>>,
    /// Track identifier to resolve. Mutually exclusive with `encoded`.
    #[serde(default)]
    pub identifier: Option<String>,
    /// User data to attach to the track.
    #[serde(default)]
    pub user_data: Option<serde_json::Value>,
}

// ─── Session Update ─────────────────────────────────────────────────────────

/// Request body for PATCH /v4/sessions/{sessionId}.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionUpdate {
    #[serde(default)]
    pub resuming: Option<bool>,
    #[serde(default)]
    pub timeout: Option<u64>,
}

/// Response from PATCH /v4/sessions/{sessionId}.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub resuming: bool,
    pub timeout: u64,
}

// ─── Filters ────────────────────────────────────────────────────────────────

/// All audio filters. Omitted fields (None) mean "not set / removed".
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

/// A single equalizer band.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EqBand {
    /// Band index (0-14).
    pub band: u8,
    /// Gain multiplier (-0.25 to 1.0). 0.0 = no change.
    pub gain: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KaraokeFilter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mono_level: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter_band: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter_width: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimescaleFilter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pitch: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TremoloFilter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depth: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VibratoFilter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depth: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DistortionFilter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sin_offset: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sin_scale: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cos_offset: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cos_scale: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tan_offset: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tan_scale: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RotationFilter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotation_hz: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelMixFilter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub left_to_left: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub left_to_right: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub right_to_left: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub right_to_right: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LowPassFilter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub smoothing: Option<f32>,
}

// ─── WebSocket Outgoing Messages ────────────────────────────────────────────

/// Messages sent from server to client over WebSocket.
#[derive(Debug, Serialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum OutgoingMessage {
    /// Sent on initial connection or resume.
    Ready {
        resumed: bool,
        #[serde(rename = "sessionId")]
        session_id: String,
    },
    /// Periodic player state update.
    #[serde(rename_all = "camelCase")]
    PlayerUpdate {
        guild_id: String,
        state: PlayerState,
    },
    /// Server statistics.
    Stats(Stats),
    /// Player event (track start, end, error, etc).
    Event(LavalinkEvent),
}

// ─── Events ─────────────────────────────────────────────────────────────────

/// Events emitted by the player, sent as WebSocket messages.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum LavalinkEvent {
    /// A track started playing.
    #[serde(rename = "TrackStartEvent")]
    #[serde(rename_all = "camelCase")]
    TrackStart { guild_id: String, track: Track },

    /// A track ended.
    #[serde(rename = "TrackEndEvent")]
    #[serde(rename_all = "camelCase")]
    TrackEnd {
        guild_id: String,
        track: Track,
        reason: TrackEndReason,
    },

    /// A track threw an exception during playback.
    #[serde(rename = "TrackExceptionEvent")]
    #[serde(rename_all = "camelCase")]
    TrackException {
        guild_id: String,
        track: Track,
        exception: TrackException,
    },

    /// A track got stuck (no audio frames for threshold duration).
    #[serde(rename = "TrackStuckEvent")]
    #[serde(rename_all = "camelCase")]
    TrackStuck {
        guild_id: String,
        track: Track,
        threshold_ms: u64,
    },

    /// The voice WebSocket connection was closed.
    #[serde(rename = "WebSocketClosedEvent")]
    #[serde(rename_all = "camelCase")]
    WebSocketClosed {
        guild_id: String,
        code: u16,
        reason: String,
        by_remote: bool,
    },
}

/// Reason a track ended.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TrackEndReason {
    Finished,
    LoadFailed,
    Stopped,
    Replaced,
    Cleanup,
}

/// Exception info for a failed track.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackException {
    pub message: Option<String>,
    pub severity: Severity,
    pub cause: String,
}

// ─── Stats ──────────────────────────────────────────────────────────────────

/// Server statistics.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    /// Total player count across all sessions.
    pub players: i32,
    /// Players currently playing audio.
    pub playing_players: i32,
    /// Server uptime in milliseconds.
    pub uptime: u64,
    pub memory: Memory,
    pub cpu: Cpu,
    /// Frame stats. Only present in WebSocket stats (not REST).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame_stats: Option<FrameStats>,
}

/// Memory statistics.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Memory {
    /// Free memory in bytes.
    pub free: u64,
    /// Used memory in bytes.
    pub used: u64,
    /// Allocated (total) memory in bytes.
    pub allocated: u64,
    /// Maximum reservable memory in bytes.
    pub reservable: u64,
}

/// CPU statistics.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Cpu {
    /// Logical core count.
    pub cores: i32,
    /// System-wide CPU load (0.0–1.0).
    pub system_load: f64,
    /// Server process CPU load normalized across cores (0.0–1.0).
    pub lavalink_load: f64,
}

/// Audio frame delivery statistics.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameStats {
    /// Average frames sent per player (last minute).
    pub sent: i32,
    /// Average null frames per player (last minute).
    pub nulled: i32,
    /// Average frame deficit per player (expected − sent − nulled).
    pub deficit: i32,
}

// ─── Info ───────────────────────────────────────────────────────────────────

/// Server information response.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Info {
    pub version: Version,
    pub build_time: u64,
    pub git: GitInfo,
    /// Runtime info. "Rust" for this implementation.
    pub jvm: String,
    /// Audio library version.
    pub lavaplayer: String,
    /// Enabled source manager names.
    pub source_managers: Vec<String>,
    /// Enabled filter names.
    pub filters: Vec<String>,
    /// Loaded plugins.
    pub plugins: Vec<PluginInfo>,
}

/// Server version.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Version {
    pub semver: String,
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pre_release: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub build: Option<String>,
}

/// Git commit info.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitInfo {
    pub branch: String,
    pub commit: String,
    pub commit_time: u64,
}

/// Plugin info.
#[derive(Debug, Serialize)]
pub struct PluginInfo {
    pub name: String,
    pub version: String,
}

// ─── Error Response ─────────────────────────────────────────────────────────

/// Lavalink v4 JSON error response format.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LavalinkError {
    /// Unix timestamp in milliseconds.
    pub timestamp: u64,
    /// HTTP status code.
    pub status: u16,
    /// HTTP status reason phrase (e.g. "Bad Request").
    pub error: String,
    /// Human-readable error message.
    pub message: String,
    /// The request path that caused the error.
    pub path: String,
    /// Stack trace (only in non-production).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace: Option<String>,
}

impl LavalinkError {
    pub fn bad_request(message: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            status: 400,
            error: "Bad Request".into(),
            message: message.into(),
            path: path.into(),
            trace: None,
        }
    }

    pub fn not_found(message: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            status: 404,
            error: "Not Found".into(),
            message: message.into(),
            path: path.into(),
            trace: None,
        }
    }
}

// ─── Routeplanner ───────────────────────────────────────────────────────────

/// Routeplanner status response, polymorphic by planner type.
#[derive(Debug, Serialize)]
#[serde(tag = "class", content = "details")]
pub enum RoutePlannerStatus {
    RotatingIpRoutePlanner(RotatingIpDetails),
    NanoIpRoutePlanner(NanoIpDetails),
    RotatingNanoIpRoutePlanner(RotatingNanoIpDetails),
    BalancingIpRoutePlanner(BalancingIpDetails),
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RotatingIpDetails {
    pub ip_block: IpBlock,
    pub failing_addresses: Vec<FailingAddress>,
    pub rotate_index: String,
    pub ip_index: String,
    pub current_address: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NanoIpDetails {
    pub ip_block: IpBlock,
    pub failing_addresses: Vec<FailingAddress>,
    pub current_address: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RotatingNanoIpDetails {
    pub ip_block: IpBlock,
    pub failing_addresses: Vec<FailingAddress>,
    pub block_index: String,
    pub current_address_index: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BalancingIpDetails {
    pub ip_block: IpBlock,
    pub failing_addresses: Vec<FailingAddress>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IpBlock {
    #[serde(rename = "type")]
    pub block_type: String,
    pub size: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FailingAddress {
    pub failing_address: String,
    pub failing_timestamp: u64,
    pub failing_time: String,
}

/// Request body for POST /v4/routeplanner/free/address.
#[derive(Debug, Deserialize)]
pub struct FreeAddressRequest {
    pub address: String,
}
