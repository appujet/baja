use serde::{Deserialize, Serialize};

/// Request parameters for the `loadtracks` endpoint.
#[derive(Deserialize)]
pub struct LoadTracksQuery {
    /// The identifier/link to load.
    pub identifier: String,
}

/// The overall response structure for track loading (used by SourceManager).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadTracksResponse {
    pub load_type: LoadType,
    pub data: LoadData,
}

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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistData {
    pub info: PlaylistInfo,
    pub tracks: Vec<Track>,
}

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

/// Source-level track representation.
#[derive(Serialize, Clone)]
pub struct Track {
    pub encoded: String,
    pub info: TrackInfo,
}

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

#[derive(Serialize)]
pub struct PlaylistInfo {
    pub name: String,
    pub selected_track: i32,
}

#[derive(Serialize)]
pub struct Exception {
    pub message: String,
    pub severity: String,
    pub cause: String,
}
