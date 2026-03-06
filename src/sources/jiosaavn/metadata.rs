use serde_json::Value;

use super::{
    JioSaavnSource,
    helpers::{clean_string, get_json},
    parser::parse_track,
};
use crate::protocol::tracks::{LoadError, LoadResult, PlaylistData, PlaylistInfo};

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
                .or_else(|| (json.get("id").is_some()).then_some(json))
        })
    }

    pub async fn resolve_list(&self, type_: &str, id: &str) -> LoadResult {
        let t = if type_ == "featured" || type_ == "s/playlist" {
            "playlist"
        } else {
            type_
        };
        let n = if type_ == "artist" {
            self.artist_load_limit
        } else if type_ == "album" {
            self.album_load_limit
        } else {
            self.playlist_load_limit
        };

        let n_str = n.to_string();
        let mut params = vec![
            ("__call", "webapi.get"),
            ("api_version", "4"),
            ("_format", "json"),
            ("_marker", "0"),
            ("ctx", "web6dot0"),
            ("token", id),
            ("type", t),
        ];

        if type_ == "artist" {
            params.push(("n_song", &n_str));
        } else {
            params.push(("n", &n_str));
        }

        if let Some(data) = get_json(&self.client, &params).await {
            let list = data
                .get("list")
                .or_else(|| data.get("topSongs"))
                .and_then(|v| v.as_array());

            if let Some(arr) = list
                && !arr.is_empty()
            {
                let tracks: Vec<_> = arr.iter().filter_map(parse_track).collect();
                let mut name = clean_string(
                    data.get("title")
                        .or_else(|| data.get("name"))
                        .and_then(|v| v.as_str())
                        .unwrap_or(""),
                );
                if type_ == "artist" {
                    name = format!("{name}'s Top Tracks");
                }

                return LoadResult::Playlist(PlaylistData {
                    info: PlaylistInfo {
                        name,
                        selected_track: -1,
                    },
                    plugin_info: serde_json::json!({
                        "url": data.get("perma_url").and_then(|v| v.as_str()),
                        "type": type_,
                        "artworkUrl": data.get("image").and_then(|v| v.as_str()).map(|s| s.replace("150x150", "500x500").replace("50x50", "500x500")),
                        "author": data.get("subtitle").or_else(|| data.get("header_desc")).and_then(|v| v.as_str()).map(|s| s.split(',').take(3).collect::<Vec<_>>().join(", ")),
                        "totalTracks": data.get("list_count").and_then(|v| v.as_str()).and_then(|s| s.parse::<u64>().ok()).unwrap_or(tracks.len() as u64)
                    }),
                    tracks,
                });
            }
            LoadResult::Empty {}
        } else {
            LoadResult::Error(LoadError {
                message: Some("JioSaavn list fetch failed".to_owned()),
                severity: crate::common::Severity::Common,
                cause: String::new(),
                cause_stack_trace: None,
            })
        }
    }
}
