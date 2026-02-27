use super::DeezerSource;
use crate::protocol::tracks::{LoadResult, PlaylistData, PlaylistInfo, Track};

impl DeezerSource {
    pub(crate) async fn search(&self, query: &str) -> LoadResult {
        let url = format!("search?q={}", urlencoding::encode(query));
        if let Some(json) = self.get_json_public(&url).await {
            if let Some(data) = json.get("data").and_then(|v| v.as_array()) {
                if data.is_empty() {
                    return LoadResult::Empty {};
                }
                let tracks: Vec<Track> = data
                    .iter()
                    .filter_map(|item| self.parse_track(item))
                    .collect();
                if tracks.is_empty() {
                    return LoadResult::Empty {};
                }
                return LoadResult::Search(tracks);
            }
        }
        LoadResult::Empty {}
    }

    pub(crate) async fn get_autocomplete(
        &self,
        query: &str,
        types: &[String],
    ) -> Option<crate::protocol::tracks::SearchResult> {
        let url = format!("search/autocomplete?q={}", urlencoding::encode(query));
        let json = self.get_json_public(&url).await?;

        let all_types = types.is_empty();

        let mut tracks = Vec::new();
        let mut albums = Vec::new();
        let mut artists = Vec::new();
        let mut playlists = Vec::new();
        let texts = Vec::new();

        if all_types || types.contains(&"album".to_string()) {
            if let Some(data) = json.pointer("/albums/data").and_then(|v| v.as_array()) {
                for album in data {
                    let title = album
                        .get("title")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Unknown Album")
                        .to_string();
                    let link = album
                        .get("link")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let cover_xl = album
                        .get("cover_xl")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let artist_name = album
                        .pointer("/artist/name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Unknown Artist")
                        .to_string();
                    let nb_tracks = album.get("nb_tracks").and_then(|v| v.as_u64()).unwrap_or(0);

                    albums.push(PlaylistData {
                        info: PlaylistInfo {
                            name: title,
                            selected_track: -1,
                        },
                        plugin_info: serde_json::json!({
                          "type": "album",
                          "url": link,
                          "artworkUrl": if cover_xl.is_empty() { None } else { Some(cover_xl) },
                          "author": artist_name,
                          "totalTracks": nb_tracks
                        }),
                        tracks: Vec::new(),
                    });
                }
            }
        }

        if all_types || types.contains(&"artist".to_string()) {
            if let Some(data) = json.pointer("/artists/data").and_then(|v| v.as_array()) {
                for artist in data {
                    let name = artist
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Unknown Artist")
                        .to_string();
                    let link = artist
                        .get("link")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let picture_xl = artist
                        .get("picture_xl")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();

                    artists.push(PlaylistData {
                        info: PlaylistInfo {
                            name: format!("{}'s Top Tracks", name),
                            selected_track: -1,
                        },
                        plugin_info: serde_json::json!({
                          "type": "artist",
                          "url": link,
                          "artworkUrl": if picture_xl.is_empty() { None } else { Some(picture_xl) },
                          "author": name,
                          "totalTracks": 0
                        }),
                        tracks: Vec::new(),
                    });
                }
            }
        }

        if all_types || types.contains(&"playlist".to_string()) {
            if let Some(data) = json.pointer("/playlists/data").and_then(|v| v.as_array()) {
                for playlist in data {
                    let title = playlist
                        .get("title")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Unknown Playlist")
                        .to_string();
                    let link = playlist
                        .get("link")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let picture_xl = playlist
                        .get("picture_xl")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let creator_name = playlist
                        .pointer("/creator/name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Unknown Creator")
                        .to_string();
                    let nb_tracks = playlist
                        .get("nb_tracks")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);

                    playlists.push(PlaylistData {
                        info: PlaylistInfo {
                            name: title,
                            selected_track: -1,
                        },
                        plugin_info: serde_json::json!({
                          "type": "playlist",
                          "url": link,
                          "artworkUrl": if picture_xl.is_empty() { None } else { Some(picture_xl) },
                          "author": creator_name,
                          "totalTracks": nb_tracks
                        }),
                        tracks: Vec::new(),
                    });
                }
            }
        }

        if all_types || types.contains(&"track".to_string()) {
            if let Some(data) = json.pointer("/tracks/data").and_then(|v| v.as_array()) {
                for track in data {
                    if let Some(parsed) = self.parse_track(track) {
                        tracks.push(parsed);
                    }
                }
            }
        }

        Some(crate::protocol::tracks::SearchResult {
            tracks,
            albums,
            artists,
            playlists,
            texts,
            plugin: serde_json::json!({}),
        })
    }
}
