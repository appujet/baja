use tracing::debug;

use super::{
    JioSaavnSource,
    helpers::get_json,
    parser::{parse_search_item, parse_search_playlist, parse_track},
};
use crate::api::tracks::{LoadError, LoadResult, SearchResult};

impl JioSaavnSource {
    pub async fn search(&self, query: &str) -> LoadResult {
        debug!("JioSaavn searching: {}", query);

        let params = vec![
            ("__call", "search.getResults"),
            ("api_version", "4"),
            ("_format", "json"),
            ("_marker", "0"),
            ("cc", "in"),
            ("ctx", "web6dot0"),
            ("includeMetaTags", "1"),
            ("q", query),
        ];

        if let Some(json) = get_json(&self.client, &params).await {
            if let Some(results) = json.get("results").and_then(|v| v.as_array()) {
                if results.is_empty() {
                    return LoadResult::Empty {};
                }
                let tracks: Vec<_> = results
                    .iter()
                    .take(self.search_limit)
                    .filter_map(|item| parse_track(item))
                    .collect();
                return LoadResult::Search(tracks);
            }
            LoadResult::Empty {}
        } else {
            LoadResult::Error(LoadError {
                message: "JioSaavn search failed".to_string(),
                severity: crate::common::Severity::Common,
                cause: "".to_string(),
            })
        }
    }

    pub async fn get_autocomplete(&self, query: &str, types: &[String]) -> Option<SearchResult> {
        debug!("JioSaavn get_autocomplete: {}", query);

        let params = vec![
            ("__call", "autocomplete.get"),
            ("api_version", "4"),
            ("_format", "json"),
            ("_marker", "0"),
            ("ctx", "web6dot0"),
            ("query", query),
        ];

        let json = get_json(&self.client, &params).await?;

        let mut tracks = Vec::new();
        let mut albums = Vec::new();
        let mut artists = Vec::new();
        let mut playlists = Vec::new();
        let texts = Vec::new();

        let all_types = types.is_empty();

        // Parse Songs -> Tracks
        if all_types || types.contains(&"track".to_string()) {
            if let Some(songs) = json
                .get("songs")
                .and_then(|v| v.get("data"))
                .and_then(|v| v.as_array())
            {
                for item in songs {
                    if let Some(track) = parse_search_item(item) {
                        tracks.push(track);
                    }
                }
            }
        }

        // Batch update track details for duration and perma_url
        if !tracks.is_empty() {
            let pids: Vec<String> = tracks.iter().map(|t| t.info.identifier.clone()).collect();
            let pids_str = pids.join(",");
            let details_params = vec![
                ("__call", "song.getDetails"),
                ("_format", "json"),
                ("pids", &pids_str),
            ];

            if let Some(details_json) = get_json(&self.client, &details_params).await {
                for track in &mut tracks {
                    if let Some(detail) = details_json.get(&track.info.identifier) {
                        // Update length
                        if let Some(duration) = detail
                            .get("duration")
                            .and_then(|v| v.as_str())
                            .and_then(|s| s.parse::<u64>().ok())
                            .or_else(|| detail.get("duration").and_then(|v| v.as_u64()))
                        {
                            track.info.length = duration * 1000;
                        }

                        // Update URI
                        if let Some(perma_url) = detail.get("perma_url").and_then(|v| v.as_str()) {
                            track.info.uri = Some(perma_url.to_string());
                        }

                        // Enrich pluginInfo
                        track.plugin_info = crate::api::tracks::PluginInfo {
                            album_name: detail
                                .get("album")
                                .or_else(|| detail.pointer("/more_info/album"))
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                            album_url: detail
                                .get("album_url")
                                .or_else(|| detail.pointer("/more_info/album_url"))
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                            artist_url: detail
                                .pointer("/more_info/artistMap/primary_artists/0/perma_url")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                            artist_artwork_url: detail
                                .pointer("/more_info/artistMap/primary_artists/0/image")
                                .and_then(|v| v.as_str())
                                .map(|s| {
                                    s.replace("150x150", "500x500").replace("50x50", "500x500")
                                }),
                            preview_url: detail
                                .get("media_preview_url")
                                .or_else(|| detail.pointer("/more_info/media_preview_url"))
                                .or_else(|| detail.get("vlink"))
                                .or_else(|| detail.pointer("/more_info/vlink"))
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                            is_preview: false,
                        };

                        // Update Author
                        if let Some(artists) =
                            detail.get("primary_artists").and_then(|v| v.as_str())
                        {
                            if !artists.is_empty() {
                                track.info.author = super::helpers::clean_string(artists);
                            }
                        }

                        // Re-encode track with updated info
                        track.encoded = track.encode();
                    }
                }
            }
        }

        // Parse Albums -> PlaylistData
        if all_types || types.contains(&"album".to_string()) {
            if let Some(data) = json
                .get("albums")
                .and_then(|v| v.get("data"))
                .and_then(|v| v.as_array())
            {
                for item in data {
                    if let Some(pd) = parse_search_playlist(item, "album") {
                        albums.push(pd);
                    }
                }
            }
        }

        // Parse Artists -> PlaylistData
        if all_types || types.contains(&"artist".to_string()) {
            if let Some(data) = json
                .get("artists")
                .and_then(|v| v.get("data"))
                .and_then(|v| v.as_array())
            {
                for item in data {
                    if let Some(pd) = parse_search_playlist(item, "artist") {
                        artists.push(pd);
                    }
                }
            }
        }

        // Parse Playlists -> PlaylistData
        if all_types || types.contains(&"playlist".to_string()) {
            if let Some(data) = json
                .get("playlists")
                .and_then(|v| v.get("data"))
                .and_then(|v| v.as_array())
            {
                for item in data {
                    if let Some(pd) = parse_search_playlist(item, "playlist") {
                        playlists.push(pd);
                    }
                }
            }
        }

        // Parse TopQuery -> Respective Type
        if all_types || types.is_empty() {
            if let Some(top_data) = json
                .get("topquery")
                .and_then(|v| v.get("data"))
                .and_then(|v| v.as_array())
            {
                for item in top_data {
                    let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
                    match item_type {
                        "song" => {
                            if let Some(track) = parse_search_item(item) {
                                if !tracks
                                    .iter()
                                    .any(|t| t.info.identifier == track.info.identifier)
                                {
                                    tracks.insert(0, track);
                                }
                            }
                        }
                        "album" => {
                            if let Some(pd) = parse_search_playlist(item, "album") {
                                if !albums.iter().any(|a| a.info.name == pd.info.name) {
                                    albums.insert(0, pd);
                                }
                            }
                        }
                        "artist" => {
                            if let Some(pd) = parse_search_playlist(item, "artist") {
                                if !artists.iter().any(|a| a.info.name == pd.info.name) {
                                    artists.insert(0, pd);
                                }
                            }
                        }
                        "playlist" => {
                            if let Some(pd) = parse_search_playlist(item, "playlist") {
                                if !playlists.iter().any(|a| a.info.name == pd.info.name) {
                                    playlists.insert(0, pd);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        Some(SearchResult {
            tracks,
            albums,
            artists,
            playlists,
            texts,
            plugin: serde_json::json!({}),
        })
    }
}
