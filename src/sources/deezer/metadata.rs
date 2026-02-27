use super::DeezerSource;
use crate::api::tracks::{LoadResult, PlaylistData, PlaylistInfo, Track};

impl DeezerSource {
    pub(crate) async fn get_track_by_isrc(&self, isrc: &str) -> Option<Track> {
        let url = format!("track/isrc:{}", isrc);
        tracing::debug!(
            "DeezerSource: Fetching metadata for ISRC: {} (URL: {})",
            isrc,
            url
        );
        let json = self.get_json_public(&url).await?;
        if json.get("id").is_some() {
            let res = self.parse_track(&json);
            if let Some(ref t) = res {
                tracing::debug!(
                    "DeezerSource: Found track for ISRC {}: {}",
                    isrc,
                    t.info.identifier
                );
            } else {
                tracing::debug!("DeezerSource: Failed to parse track for ISRC {}", isrc);
            }
            res
        } else {
            tracing::debug!("DeezerSource: No track found for ISRC {}", isrc);
            None
        }
    }

    pub(crate) async fn get_album(&self, id: &str) -> LoadResult {
        let json = match self.get_json_public(&format!("album/{}", id)).await {
            Some(j) => j,
            None => return LoadResult::Empty {},
        };
        let tracks_json = match self
            .get_json_public(&format!("album/{}/tracks?limit=10000", id))
            .await
        {
            Some(j) => j,
            None => return LoadResult::Empty {},
        };
        let mut tracks = Vec::new();
        let artwork_url = json
            .get("cover_xl")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        if let Some(data) = tracks_json.get("data").and_then(|d| d.as_array()) {
            for item in data {
                if let Some(mut track) = self.parse_track(item) {
                    if track.info.artwork_url.is_none() {
                        track.info.artwork_url = artwork_url.clone();
                    }
                    tracks.push(track);
                }
            }
        }
        if tracks.is_empty() {
            return LoadResult::Empty {};
        }
        LoadResult::Playlist(PlaylistData {
            info: PlaylistInfo {
                name: json
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown Album")
                    .to_string(),
                selected_track: -1,
            },
            plugin_info: serde_json::json!({
              "type": "album",
              "url": format!("https://www.deezer.com/album/{}", id),
              "artworkUrl": json.get("cover_xl").and_then(|v| v.as_str()),
              "author": json.get("artist").and_then(|v| v.get("name")).and_then(|v| v.as_str()),
              "totalTracks": json.get("nb_tracks").and_then(|v| v.as_u64()).unwrap_or(tracks.len() as u64)
            }),
            tracks,
        })
    }

    pub(crate) async fn get_playlist(&self, id: &str) -> LoadResult {
        let json = match self.get_json_public(&format!("playlist/{}", id)).await {
            Some(j) => j,
            None => return LoadResult::Empty {},
        };
        let tracks_json = match self
            .get_json_public(&format!("playlist/{}/tracks?limit=10000", id))
            .await
        {
            Some(j) => j,
            None => return LoadResult::Empty {},
        };
        let mut tracks = Vec::new();
        if let Some(data) = tracks_json.get("data").and_then(|d| d.as_array()) {
            for item in data {
                if let Some(track) = self.parse_track(item) {
                    tracks.push(track);
                }
            }
        }
        if tracks.is_empty() {
            return LoadResult::Empty {};
        }
        LoadResult::Playlist(PlaylistData {
            info: PlaylistInfo {
                name: json
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown Playlist")
                    .to_string(),
                selected_track: -1,
            },
            plugin_info: serde_json::json!({
              "type": "playlist",
              "url": format!("https://www.deezer.com/playlist/{}", id),
              "artworkUrl": json.get("picture_xl").and_then(|v| v.as_str()),
              "author": json.get("creator").and_then(|v| v.get("name")).and_then(|v| v.as_str()),
              "totalTracks": json.get("nb_tracks").and_then(|v| v.as_u64()).unwrap_or(tracks.len() as u64)
            }),
            tracks,
        })
    }

    pub(crate) async fn get_artist(&self, id: &str) -> LoadResult {
        let json = match self.get_json_public(&format!("artist/{}", id)).await {
            Some(j) => j,
            None => return LoadResult::Empty {},
        };
        let tracks_json = match self
            .get_json_public(&format!("artist/{}/top?limit=50", id))
            .await
        {
            Some(j) => j,
            None => return LoadResult::Empty {},
        };
        let artwork_url = json
            .get("picture_xl")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let author = json
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown Artist")
            .to_string();
        let mut tracks = Vec::new();
        if let Some(data) = tracks_json.get("data").and_then(|d| d.as_array()) {
            for item in data {
                if let Some(mut track) = self.parse_track(item) {
                    if track.info.artwork_url.is_none() {
                        track.info.artwork_url = artwork_url.clone();
                    }
                    tracks.push(track);
                }
            }
        }
        if tracks.is_empty() {
            return LoadResult::Empty {};
        }
        LoadResult::Playlist(PlaylistData {
            info: PlaylistInfo {
                name: format!("{}'s Top Tracks", author),
                selected_track: -1,
            },
            plugin_info: serde_json::json!({
              "type": "artist",
              "url": format!("https://www.deezer.com/artist/{}", id),
              "artworkUrl": json.get("picture_xl").and_then(|v| v.as_str()),
              "author": author,
              "totalTracks": tracks.len()
            }),
            tracks,
        })
    }
}
