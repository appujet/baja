use serde::{Deserialize, Serialize};

use crate::protocol::codec::{decode_track, encode_track};

/// A single audio track with encoded data and metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    /// Base64-encoded track data.
    pub encoded: String,
    /// Track metadata.
    pub info: TrackInfo,
    /// Plugin-specific info — free JSON object whose shape is defined by the plugin.
    #[serde(default = "serde_json::Value::default")]
    pub plugin_info: serde_json::Value,
    /// User-provided data attached to the track.
    #[serde(default = "serde_json::Value::default")]
    pub user_data: serde_json::Value,
}
impl Track {
    /// Create a new Track from info and encode it.
    pub fn new(info: TrackInfo) -> Self {
        let mut track = Self {
            encoded: String::new(),
            info,
            plugin_info: serde_json::json!({}),
            user_data: serde_json::json!({}),
        };
        track.encoded = track.encode();
        track
    }

    /// Encode the track into a base64 string.
    pub fn encode(&self) -> String {
        encode_track(&self.info, &self.user_data).unwrap_or_else(|_| self.encoded.clone())
    }

    /// Decode a track from a base64 string.
    pub fn decode(encoded: &str) -> Option<Self> {
        decode_track(encoded).ok()
    }
}

/// Metadata for an audio track.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TrackInfo {
    pub identifier: String,
    pub is_seekable: bool,
    pub author: String,
    /// Duration in milliseconds. 0 for live streams.
    pub length: u64,
    pub is_stream: bool,
    /// Current playback position in milliseconds.
    pub position: u64,
    pub title: String,
    pub uri: Option<String>,
    pub artwork_url: Option<String>,
    pub isrc: Option<String>,
    pub source_name: String,
}

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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistData {
    pub info: PlaylistInfo,
    pub plugin_info: serde_json::Value,
    pub tracks: Vec<Track>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextData {
    pub text: String,
    pub plugin: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub tracks: Vec<Track>,
    pub albums: Vec<PlaylistData>,
    pub artists: Vec<PlaylistData>,
    pub playlists: Vec<PlaylistData>,
    pub texts: Vec<TextData>,
    pub plugin: serde_json::Value,
}

/// Playlist metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// Human-readable error message.
    pub message: Option<String>,
    /// How severe the error is.
    pub severity: crate::common::Severity,
    /// Exception class / short cause description.
    pub cause: String,
    /// Full stack trace, if available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause_stack_trace: Option<String>,
}
