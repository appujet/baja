use serde::{Deserialize, Serialize};

/// Request parameters for the `loadtracks` endpoint.
#[derive(Deserialize)]
pub struct LoadTracksQuery {
    /// The identifier/link to load.
    pub identifier: String,
}

/// The overall response structure for track loading.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadTracksResponse {
    /// Type of result (track, playlist, search, etc.)
    pub load_type: LoadType,
    /// The actual data payload based on the load type.
    pub data: LoadData,
}

/// Untagged enum representing different payloads for track loading.
#[derive(Serialize)]
#[serde(untagged)]
pub enum LoadData {
    Track(Track),
    #[allow(dead_code)]
    Tracks(Vec<Track>),
    #[allow(dead_code)]
    Playlist(PlaylistData),
    Empty(serde_json::Value),
    #[allow(dead_code)]
    Error(Exception),
}

/// Data for a playlist result.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistData {
    pub info: PlaylistInfo,
    pub tracks: Vec<Track>,
}

/// Enum for the type of track loading result.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LoadType {
    Track,
    #[allow(dead_code)]
    Playlist,
    #[allow(dead_code)]
    Search,
    Empty,
    #[allow(dead_code)]
    Error,
}

/// Represents a single audio track.
#[derive(Serialize, Clone)]
pub struct Track {
    /// Base64 encoded track data.
    pub encoded: String,
    /// Metadata about the track.
    pub info: TrackInfo,
}

/// Metadata for an audio track.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TrackInfo {
    pub identifier: String,
    pub is_seekable: bool,
    pub author: String,
    pub length: u64,
    pub is_stream: bool,
    pub position: u64,
    pub title: String,
    pub uri: String,
    pub source_name: String,
    pub artwork_url: Option<String>,
    pub isrc: Option<String>,
}

/// Metadata for a playlist.
#[derive(Serialize)]
pub struct PlaylistInfo {
    pub name: String,
    pub selected_track: i32,
}

/// Error information for failed requests.
#[derive(Serialize)]
pub struct Exception {
    pub message: String,
    pub severity: String,
    pub cause: String,
}

/// Response for the `info` endpoint.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InfoResponse {
    pub version: Version,
    pub build_time: u64,
    pub git: GitInfo,
    pub jvm: String,
    pub lavaplayer: String,
    pub source_managers: Vec<String>,
    pub filters: Vec<String>,
    pub plugins: Vec<PluginInfo>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Version {
    pub semver: String,
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    pub pre_release: Option<String>,
    pub build: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitInfo {
    pub branch: String,
    pub commit: String,
    pub commit_time: u64,
}

#[derive(Serialize)]
pub struct PluginInfo {
    pub name: String,
    pub version: String,
}

/// Request body for updating a player.
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PlayerUpdateRequest {
    pub track: Option<PlayerUpdateTrack>,
    pub position: Option<u64>,
    #[allow(dead_code)]
    pub end_time: Option<u64>,
    pub volume: Option<u32>,
    pub paused: Option<bool>,
    pub voice: Option<PlayerUpdateVoice>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PlayerUpdateTrack {
    #[allow(dead_code)]
    pub encoded: Option<String>,
    pub identifier: Option<String>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PlayerUpdateVoice {
    pub token: String,
    pub endpoint: String,
    pub session_id: String,
    pub channel_id: Option<String>,
}

/// Full response for player state.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerResponse {
    pub guild_id: String,
    pub track: Option<Track>,
    pub volume: u32,
    pub paused: bool,
    pub state: PlayerStateResponse,
    pub voice: VoiceStateResponse,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerStateResponse {
    pub time: u64,
    pub position: u64,
    pub connected: bool,
    pub ping: i32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceStateResponse {
    pub token: String,
    pub endpoint: String,
    pub session_id: String,
}
