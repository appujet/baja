use std::{collections::BTreeMap, sync::Arc};

use async_trait::async_trait;
use rand::{Rng, distributions::Alphanumeric, thread_rng};
use regex::Regex;
use serde_json::Value;
use tracing::{error, warn};

use super::{track::AudiomackTrack, utils::build_auth_header};
use crate::{
    protocol::tracks::{LoadResult, PlaylistData, PlaylistInfo, Track, TrackInfo},
    sources::plugin::{PlayableTrack, SourcePlugin},
};

const API_BASE: &str = "https://api.audiomack.com/v1";

pub struct AudiomackSource {
    client: Arc<reqwest::Client>,
    song_regex: Regex,
    album_regex: Regex,
    playlist_regex: Regex,
    artist_regex: Regex,
    likes_regex: Regex,
    search_prefixes: Vec<String>,
    search_limit: usize,
}

impl AudiomackSource {
    pub fn new(
        config: Option<crate::configs::AudiomackConfig>,
        client: Arc<reqwest::Client>,
    ) -> Result<Self, String> {
        let search_limit = config.map(|c| c.search_limit).unwrap_or(20);

        Ok(Self {
            client,
            song_regex: Regex::new(
                r"https?://(?:www\.)?audiomack\.com/(?P<artist>[^/]+)/song/(?P<slug>[^/?#]+)",
            )
            .unwrap(),
            album_regex: Regex::new(
                r"https?://(?:www\.)?audiomack\.com/(?P<artist>[^/]+)/album/(?P<slug>[^/?#]+)",
            )
            .unwrap(),
            playlist_regex: Regex::new(
                r"https?://(?:www\.)?audiomack\.com/(?P<artist>[^/]+)/playlist/(?P<slug>[^/?#]+)",
            )
            .unwrap(),
            artist_regex: Regex::new(
                r"https?://(?:www\.)?audiomack\.com/(?P<artist>[^/?#]+)(?:/songs)?/?$",
            )
            .unwrap(),
            likes_regex: Regex::new(r"https?://(?:www\.)?audiomack\.com/(?P<artist>[^/]+)/likes")
                .unwrap(),
            search_prefixes: vec!["amksearch:".to_string()],
            search_limit,
        })
    }

    fn generate_nonce(&self) -> String {
        thread_rng()
            .sample_iter(&Alphanumeric)
            .take(32)
            .map(char::from)
            .collect()
    }

    async fn make_request(
        &self,
        method: reqwest::Method,
        endpoint: &str,
        query_params: Option<BTreeMap<String, String>>,
    ) -> Option<Value> {
        let url = format!("{}{}", API_BASE, endpoint);
        tracing::debug!(
            "Audiomack request: {} {} params: {:?}",
            method,
            url,
            query_params
        );

        let mut request_builder = self.base_request(self.client.request(method.clone(), &url));

        let nonce = self.generate_nonce();
        let timestamp = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs())
        .to_string();

        let auth_header = build_auth_header(
            method.as_str(),
            &url,
            query_params.as_ref().unwrap_or(&BTreeMap::new()),
            &nonce,
            &timestamp,
        );
        request_builder = request_builder.header("Authorization", auth_header);

        if let Some(qp) = query_params {
            request_builder = request_builder.query(&qp);
        }

        let resp = match request_builder.send().await {
            Ok(r) => r,
            Err(e) => {
                error!("Audiomack request failed: {}", e);
                return None;
            }
        };

        let status = resp.status();
        let text = match resp.text().await {
            Ok(t) => t,
            Err(e) => {
                error!("Failed to read Audiomack response text: {}", e);
                return None;
            }
        };

        if !status.is_success() {
            warn!(
                "Audiomack API error status: {} for endpoint: {}",
                status, endpoint
            );
            return None;
        }

        match serde_json::from_str(&text) {
            Ok(v) => Some(v),
            Err(e) => {
                error!("Failed to parse Audiomack JSON: {} body: {}", e, text);
                None
            }
        }
    }

    fn base_request(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        builder
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36")
            .header("Accept", "application/json")
            .header("Accept-Language", "en-US,en;q=0.9")
            .header("Origin", "https://audiomack.com")
            .header("Referer", "https://audiomack.com/")
            .header("Sec-Fetch-Site", "same-site")
            .header("Sec-Fetch-Mode", "cors")
            .header("Sec-Fetch-Dest", "empty")
            .header("Priority", "u=1, i")
            .header("DNT", "1")
            .header("sec-ch-ua-platform", "\"Windows\"")
    }

    fn parse_track(&self, json: &Value) -> Option<Track> {
        let id_val = json.get("id").or_else(|| json.get("song_id"));
        let id = if let Some(id) = id_val.and_then(|v| v.as_str()) {
            id.to_string()
        } else if let Some(id) = id_val.and_then(|v| v.as_i64()) {
            id.to_string()
        } else if let Some(id) = id_val.and_then(|v| v.as_u64()) {
            id.to_string()
        } else {
            tracing::debug!("Audiomack track missing id or song_id: {:?}", json);
            return None;
        };

        let title = match json.get("title").and_then(|v| v.as_str()) {
            Some(t) => t.to_string(),
            None => {
                tracing::debug!("Audiomack track missing title: {:?}", json);
                return None;
            }
        };

        let author = match json.get("artist").and_then(|v| v.as_str()) {
            Some(a) => a.to_string(),
            None => {
                tracing::debug!("Audiomack track missing artist: {:?}", json);
                return None;
            }
        };

        let duration_sec = if let Some(d) = json.get("duration").and_then(|v| v.as_u64()) {
            d
        } else if let Some(d) = json.get("duration").and_then(|v| v.as_i64()) {
            d as u64
        } else if let Some(d) = json
            .get("duration")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<u64>().ok())
        {
            d
        } else {
            0
        };
        let length = duration_sec * 1000;

        let uploader_slug = json
            .pointer("/uploader/url_slug")
            .and_then(|v| v.as_str())
            .or_else(|| json.get("uploader_url_slug").and_then(|v| v.as_str()))
            .unwrap_or("unknown");
        let url_slug = match json.get("url_slug").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => {
                tracing::debug!("Audiomack track missing url_slug: {:?}", json);
                return None;
            }
        };
        let uri = Some(format!(
            "https://audiomack.com/{}/song/{}",
            uploader_slug, url_slug
        ));

        let artwork_url = json
            .get("image")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let isrc = json
            .get("isrc")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        let track_info = TrackInfo {
            identifier: id,
            is_seekable: true,
            author,
            length,
            is_stream: false,
            position: 0,
            title,
            uri,
            artwork_url,
            isrc,
            source_name: "audiomack".to_string(),
        };

        Some(Track::new(track_info))
    }

    async fn search(&self, query: &str) -> LoadResult {
        let mut params = BTreeMap::new();
        params.insert("q".to_string(), query.to_string());
        params.insert("limit".to_string(), self.search_limit.to_string());
        params.insert("show".to_string(), "songs".to_string());
        params.insert("sort".to_string(), "popular".to_string());

        if let Some(json) = self
            .make_request(reqwest::Method::GET, "/search", Some(params))
            .await
        {
            if let Some(results) = json.get("results").and_then(|v| v.as_array()) {
                let tracks: Vec<Track> = results
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

    async fn get_song(&self, artist: &str, slug: &str) -> LoadResult {
        let endpoint = format!("/music/song/{}/{}", artist, slug);
        if let Some(json) = self
            .make_request(reqwest::Method::GET, &endpoint, None)
            .await
        {
            if let Some(track) = json.get("results").and_then(|v| self.parse_track(v)) {
                return LoadResult::Track(track);
            }
        }
        LoadResult::Empty {}
    }

    async fn get_playlist_items(&self, type_: &str, artist: &str, slug: &str) -> LoadResult {
        let endpoint = if type_ == "playlist" {
            format!("/playlist/{}/{}", artist, slug)
        } else {
            format!("/music/album/{}/{}", artist, slug)
        };

        if let Some(json) = self
            .make_request(reqwest::Method::GET, &endpoint, None)
            .await
        {
            if let Some(results) = json.get("results") {
                let name = results
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown")
                    .to_string();
                let tracks_val = results.get("tracks");

                if tracks_val.is_none() {
                    tracing::debug!(
                        "Audiomack {} results missing tracks field: {:?}",
                        type_,
                        results
                    );
                }

                let tracks: Vec<Track> = tracks_val
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|item| self.parse_track(item))
                            .collect()
                    })
                    .unwrap_or_default();

                if tracks.is_empty() {
                    tracing::debug!("Audiomack {} parsed 0 tracks: {:?}", type_, results);
                    return LoadResult::Empty {};
                }

                return LoadResult::Playlist(PlaylistData {
                    info: PlaylistInfo {
                        name,
                        selected_track: -1,
                    },
                    plugin_info: serde_json::json!({ "type": type_, "url": results.get("url").and_then(|v| v.as_str()).map(|s| format!("https://audiomack.com{}", s)).or_else(|| Some(format!("https://audiomack.com/{}/{}/{}", artist, type_, slug))), "artworkUrl": results.get("image").and_then(|v| v.as_str()), "author": results.get("artist").and_then(|v| v.as_str()), "totalTracks": tracks.len() }),
                    tracks,
                });
            } else {
                tracing::debug!(
                    "Audiomack {} response missing results field: {:?}",
                    type_,
                    json
                );
            }
        }
        LoadResult::Empty {}
    }

    async fn get_artist(&self, artist_slug: &str) -> LoadResult {
        if let Some(json) = self
            .make_request(
                reqwest::Method::GET,
                &format!("/artist/{}", artist_slug),
                None,
            )
            .await
        {
            let Some(results) = json.get("results") else {
                return LoadResult::Empty {};
            };
            let artist_id = if let Some(id) = results.get("id").and_then(|v| v.as_str()) {
                id.to_string()
            } else if let Some(id) = results.get("id").and_then(|v| v.as_i64()) {
                id.to_string()
            } else {
                return LoadResult::Empty {};
            };
            let Some(name) = results.get("name").and_then(|v| v.as_str()) else {
                return LoadResult::Empty {};
            };

            let mut params = BTreeMap::new();
            params.insert("artist_id".to_string(), artist_id.to_string());
            params.insert("limit".to_string(), "100".to_string());
            params.insert("sort".to_string(), "rank".to_string());
            params.insert("type".to_string(), "songs".to_string());

            if let Some(tracks_json) = self
                .make_request(reqwest::Method::GET, "/search_artist_content", Some(params))
                .await
            {
                if let Some(track_results) = tracks_json.get("results").and_then(|v| v.as_array()) {
                    let tracks: Vec<Track> = track_results
                        .iter()
                        .filter_map(|item| self.parse_track(item))
                        .collect();

                    if tracks.is_empty() {
                        return LoadResult::Empty {};
                    }

                    return LoadResult::Playlist(PlaylistData {
                        info: PlaylistInfo {
                            name: format!("{}'s Top Tracks", name),
                            selected_track: -1,
                        },
                        plugin_info: serde_json::json!({ "type": "artist", "url": results.get("url").and_then(|v| v.as_str()).map(|s| format!("https://audiomack.com{}", s)).or_else(|| Some(format!("https://audiomack.com/{}", artist_slug))), "artworkUrl": results.get("image").and_then(|v| v.as_str()), "author": name, "totalTracks": tracks.len() }),
                        tracks,
                    });
                }
            }
        }
        LoadResult::Empty {}
    }

    async fn get_artist_likes(&self, artist_slug: &str) -> LoadResult {
        if let Some(json) = self
            .make_request(
                reqwest::Method::GET,
                &format!("/artist/{}", artist_slug),
                None,
            )
            .await
        {
            let Some(results) = json.get("results") else {
                return LoadResult::Empty {};
            };
            let Some(name) = results.get("name").and_then(|v| v.as_str()) else {
                return LoadResult::Empty {};
            };

            if let Some(likes_json) = self
                .make_request(
                    reqwest::Method::GET,
                    &format!("/artist/{}/favorites", artist_slug),
                    None,
                )
                .await
            {
                if let Some(like_results) = likes_json.get("results").and_then(|v| v.as_array()) {
                    let tracks: Vec<Track> = like_results
                        .iter()
                        .filter_map(|item| self.parse_track(item))
                        .collect();

                    if tracks.is_empty() {
                        return LoadResult::Empty {};
                    }

                    return LoadResult::Playlist(PlaylistData {
                        info: PlaylistInfo {
                            name: format!("{}'s Liked Tracks", name),
                            selected_track: -1,
                        },
                        plugin_info: serde_json::json!({}),
                        tracks,
                    });
                }
            }
        }
        LoadResult::Empty {}
    }
}

#[async_trait]
impl SourcePlugin for AudiomackSource {
    fn name(&self) -> &str {
        "audiomack"
    }

    fn can_handle(&self, identifier: &str) -> bool {
        self.search_prefixes
            .iter()
            .any(|p| identifier.starts_with(p))
            || self.song_regex.is_match(identifier)
            || self.album_regex.is_match(identifier)
            || self.playlist_regex.is_match(identifier)
            || self.artist_regex.is_match(identifier)
            || self.likes_regex.is_match(identifier)
    }

    fn search_prefixes(&self) -> Vec<&str> {
        self.search_prefixes.iter().map(|s| s.as_str()).collect()
    }

    async fn load(
        &self,
        identifier: &str,
        _routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> LoadResult {
        if let Some(prefix) = self
            .search_prefixes
            .iter()
            .find(|p| identifier.starts_with(*p))
        {
            let query = identifier.strip_prefix(prefix).unwrap();
            return self.search(query).await;
        }

        if let Some(caps) = self.song_regex.captures(identifier) {
            let artist = caps.name("artist").map(|m| m.as_str()).unwrap_or("");
            let slug = caps.name("slug").map(|m| m.as_str()).unwrap_or("");
            return self.get_song(artist, slug).await;
        }

        if let Some(caps) = self.album_regex.captures(identifier) {
            let artist = caps.name("artist").map(|m| m.as_str()).unwrap_or("");
            let slug = caps.name("slug").map(|m| m.as_str()).unwrap_or("");
            return self.get_playlist_items("album", artist, slug).await;
        }

        if let Some(caps) = self.playlist_regex.captures(identifier) {
            let artist = caps.name("artist").map(|m| m.as_str()).unwrap_or("");
            let slug = caps.name("slug").map(|m| m.as_str()).unwrap_or("");
            return self.get_playlist_items("playlist", artist, slug).await;
        }

        if let Some(caps) = self.likes_regex.captures(identifier) {
            let artist = caps.name("artist").map(|m| m.as_str()).unwrap_or("");
            return self.get_artist_likes(artist).await;
        }

        if let Some(caps) = self.artist_regex.captures(identifier) {
            let artist = caps.name("artist").map(|m| m.as_str()).unwrap_or("");
            return self.get_artist(artist).await;
        }

        LoadResult::Empty {}
    }

    async fn get_track(
        &self,
        identifier: &str,
        routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<Box<dyn PlayableTrack>> {
        let mut track_id = identifier.to_string();

        // If it's a URL, we need to get the numeric ID first
        if self.song_regex.is_match(identifier) {
            if let LoadResult::Track(track) = self.load(identifier, None).await {
                track_id = track.info.identifier;
            } else {
                return None;
            }
        }

        Some(Box::new(AudiomackTrack {
            client: self.client.clone(),
            identifier: track_id,
            local_addr: routeplanner.and_then(|rp| rp.get_address()),
        }))
    }

    fn get_proxy_config(&self) -> Option<crate::configs::HttpProxyConfig> {
        None
    }
}
