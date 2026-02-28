use serde::{Deserialize, Serialize};

/// Request parameters for the `loadtracks` endpoint.
#[derive(Deserialize)]
pub struct LoadTracksQuery {
    /// The identifier/link to load.
    pub identifier: String,
}

/// Request parameters for the `loadsearch` endpoint.
#[derive(Deserialize)]
pub struct LoadSearchQuery {
    /// The search query
    pub query: String,
    /// Comma-separated list of types to search for (e.g. "track,playlist,album,artist,text")
    pub types: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DecodeTrackQuery {
    pub encoded_track: Option<String>,
    pub track: Option<String>,
}

#[derive(Deserialize)]
pub struct EncodedTracks {
    pub tracks: Vec<String>,
}

#[derive(Serialize)]
pub struct Tracks {
    pub tracks: Vec<crate::protocol::tracks::Track>,
}

#[derive(Serialize)]
pub struct Exception {
    pub message: String,
    pub severity: String,
    pub cause: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct LyricsLine {
    pub text: String,
    pub timestamp: u64,
    pub duration: u64,
}

/// Internal lyrics data structure.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct LyricsData {
    pub name: String,
    pub author: String,
    pub provider: String,
    pub text: String,
    pub lines: Option<Vec<LyricsLine>>,
}

/// Result of a lyrics load operation.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "loadType", content = "data", rename_all = "camelCase")]
pub enum LyricsLoadResult {
    /// Synced or line-based lyrics.
    Lyrics(LyricsResultData),
    /// Plain text lyrics.
    Text(LyricsTextData),
    /// No lyrics found.
    Empty {},
    /// An error occurred during loading.
    Error(LyricsLoadError),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LyricsResultData {
    pub name: String,
    pub synced: bool,
    pub lines: Vec<LyricsLine>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LyricsTextData {
    pub text: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LyricsLoadError {
    pub message: String,
    pub severity: crate::common::Severity,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RustalinkLyrics {
    pub source_name: String,
    pub provider: Option<String>,
    pub text: Option<String>,
    pub lines: Option<Vec<RustalinkLyricsLine>>,
    pub plugin: serde_json::Value,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RustalinkLyricsLine {
    pub timestamp: u64,
    pub duration: Option<u64>,
    pub line: String,
    pub plugin: serde_json::Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetLyricsQuery {
    pub track: String,
    #[serde(default)]
    pub skip_track_source: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetPlayerLyricsQuery {
    #[serde(default)]
    pub skip_track_source: bool,
}
