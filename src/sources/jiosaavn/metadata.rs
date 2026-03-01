use serde_json::Value;

use super::{JioSaavnSource, helpers::get_json, parser::parse_track};
use crate::protocol::tracks::{LoadError, LoadResult};

impl JioSaavnSource {
    pub async fn fetch_metadata(&self, id: &str) -> Option<Value> {
        let params = vec![
            ("__call", "webapi.get"),
            ("api_version", "4"),
            ("_format", "json"),
            ("_marker", "0"),
            ("ctx", "web6dot0"),
            ("token", id),
            ("type", "song"),
        ];

        get_json(&self.client, &params).await.and_then(|json| {
            json.get("songs")
                .and_then(|s| s.get(0))
                .cloned()
                .or_else(|| {
                    if json.get("id").is_some() {
                        Some(json)
                    } else {
                        None
                    }
                })
        })
    }

    pub async fn resolve_list(&self, type_: &str, id: &str) -> LoadResult {
        let type_param = if type_ == "featured" || type_ == "s/playlist" {
            "playlist"
        } else {
            type_
        };

        let n_songs = if type_ == "artist" {
            self.artist_load_limit
        } else if type_ == "album" {
            self.album_load_limit
        } else {
            self.playlist_load_limit
        };
        let n_songs_str = n_songs.to_string();

        let mut params = vec![
            ("__call", "webapi.get"),
            ("api_version", "4"),
            ("_format", "json"),
            ("_marker", "0"),
            ("ctx", "web6dot0"),
            ("token", id),
            ("type", type_param),
        ];

        if type_ == "artist" {
            params.push(("n_song", &n_songs_str));
        } else {
            params.push(("n", &n_songs_str));
        }

        if let Some(data) = get_json(&self.client, &params).await {
            let list = data.get("list").or_else(|| data.get("topSongs"));
            if let Some(arr) = list.and_then(|v| v.as_array()) {
                if arr.is_empty() {
                    return LoadResult::Empty {};
                }

                let tracks: Vec<_> = arr.iter().filter_map(|item| parse_track(item)).collect();

                let name_raw = data
                    .get("title")
                    .or_else(|| data.get("name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let mut name = crate::sources::jiosaavn::helpers::clean_string(name_raw);

                if type_ == "artist" {
                    name = format!("{}'s Top Tracks", name);
                }

                LoadResult::Playlist(crate::protocol::tracks::PlaylistData {
                    info: crate::protocol::tracks::PlaylistInfo {
                        name,
                        selected_track: -1,
                    },
                    plugin_info: serde_json::json!({
                      "url": data.get("perma_url").and_then(|v| v.as_str()),
                      "type": type_,
                      "artworkUrl": data.get("image").and_then(|v| v.as_str()).map(|s| s.replace("150x150", "500x500").replace("50x50", "500x500")),
                      "author": data.get("subtitle").or_else(|| data.get("header_desc")).and_then(|v| v.as_str()).map(|s| s.split(',').map(|p| p.trim()).take(3).collect::<Vec<_>>().join(", ")),
                      "totalTracks": data.get("list_count").and_then(|v| v.as_str()).and_then(|s| s.parse::<u64>().ok()).unwrap_or(tracks.len() as u64)
                    }),
                    tracks,
                })
            } else {
                LoadResult::Empty {}
            }
        } else {
            LoadResult::Error(LoadError {
                message: Some("JioSaavn list fetch failed".to_string()),
                severity: crate::common::Severity::Common,
                cause: String::new(),
                cause_stack_trace: None,
            })
        }
    }
}
