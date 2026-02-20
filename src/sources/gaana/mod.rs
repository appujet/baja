pub mod crypto;

use crate::api::tracks::{LoadError, LoadResult, PlaylistData, PlaylistInfo, Track, TrackInfo};
use crate::sources::SourcePlugin;
use async_trait::async_trait;
use regex::Regex;
use reqwest::header::HeaderMap;
use serde_json::Value;
use std::sync::Arc;
use tracing::{debug, warn};

const API_URL: &str = "https://gaana.com/apiv2";
const STREAM_URL_API: &str = "https://gaana.com/api/stream-url";
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.0.0 Safari/537.36";

pub struct GaanaSource {
    client: reqwest::Client,
    url_regex: Regex,
    search_prefix: String,
    stream_quality: String,
    proxy: Option<crate::configs::HttpProxyConfig>,
    // Limits
    search_limit: usize,
    playlist_load_limit: usize,
    album_load_limit: usize,
    artist_load_limit: usize,
}

impl GaanaSource {
    pub fn new(config: Option<crate::configs::GaanaConfig>) -> Self {
        let mut headers = HeaderMap::new();
        headers.insert("User-Agent", USER_AGENT.parse().unwrap());
        headers.insert("Accept", "application/json, text/plain, */*".parse().unwrap());
        headers.insert("Accept-Language", "en-US,en;q=0.9".parse().unwrap());
        headers.insert("Accept-Encoding", "gzip, deflate, br".parse().unwrap());
        headers.insert("Origin", "https://gaana.com".parse().unwrap());
        headers.insert("Referer", "https://gaana.com/".parse().unwrap());
        headers.insert("Connection", "keep-alive".parse().unwrap());
        headers.insert("Sec-Fetch-Dest", "empty".parse().unwrap());
        headers.insert("Sec-Fetch-Mode", "cors".parse().unwrap());
        headers.insert("Sec-Fetch-Site", "same-origin".parse().unwrap());
        headers.insert("sec-ch-ua", "\"Chromium\";v=\"136\", \"Google Chrome\";v=\"136\", \"Not.A/Brand\";v=\"99\"".parse().unwrap());
        headers.insert("sec-ch-ua-mobile", "?0".parse().unwrap());
        headers.insert("sec-ch-ua-platform", "\"Windows\"".parse().unwrap());

        let (
            stream_quality,
            search_limit,
            playlist_load_limit,
            album_load_limit,
            artist_load_limit,
            proxy,
        ) = if let Some(c) = config {
            (
                c.stream_quality.unwrap_or_else(|| "high".to_string()),
                c.search_limit,
                c.playlist_load_limit,
                c.album_load_limit,
                c.artist_load_limit,
                c.proxy,
            )
        } else {
            ("high".to_string(), 10, 50, 50, 20, None)
        };

        let mut client_builder = reqwest::Client::builder().default_headers(headers);

        if let Some(proxy_config) = &proxy {
            if let Some(url) = &proxy_config.url {
                debug!("Configuring proxy for GaanaSource: {}", url);
                if let Ok(proxy_obj) = reqwest::Proxy::all(url) {
                    let mut proxy_obj = proxy_obj;
                    if let (Some(username), Some(password)) =
                        (&proxy_config.username, &proxy_config.password)
                    {
                        proxy_obj = proxy_obj.basic_auth(username, password);
                    }
                    client_builder = client_builder.proxy(proxy_obj);
                }
            }
        }

        let client = client_builder.build().unwrap();

        Self {
            client,
            url_regex: Regex::new(
                r"(?:https?://)?(?:www\.)?gaana\.com/(?P<type>song|album|playlist|artist)/(?P<seokey>[\w-]+)",
            )
            .unwrap(),
            search_prefix: "gnsearch:".to_string(),
            stream_quality,
            proxy,
            search_limit,
            playlist_load_limit,
            album_load_limit,
            artist_load_limit,
        }
    }

    /// POST to Gaana API v2 and parse JSON response.
    async fn get_json(&self, params: &[(&str, &str)], referer_path: &str) -> Option<Value> {
        let url = format!(
            "{}?{}",
            API_URL,
            params
                .iter()
                .map(|(k, v)| format!("{}={}", k, urlencoding::encode(v)))
                .collect::<Vec<_>>()
                .join("&")
        );

        debug!("Gaana API request: {}", url);

        let resp = match self
            .client
            .post(&url)
            .header("Referer", format!("https://gaana.com/{}", referer_path))
            .header("Content-Length", "0")
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                warn!("Gaana API request failed: {}", e);
                return None;
            }
        };

        if !resp.status().is_success() {
            warn!("Gaana API error status: {}", resp.status());
            return None;
        }

        let text = match resp.text().await {
            Ok(t) => t,
            Err(e) => {
                warn!("Gaana: failed to read response body: {}", e);
                return None;
            }
        };

        serde_json::from_str(&text).ok()
    }

    async fn load_song(&self, seokey: &str) -> LoadResult {
        let params = [
            ("type", "songDetail"),
            ("seokey", seokey),
        ];

        let data = match self.get_json(&params, &format!("song/{}", seokey)).await {
            Some(d) => d,
            None => return LoadResult::Empty {},
        };

        let tracks = match data.get("tracks").and_then(|v| v.as_array()) {
            Some(arr) if !arr.is_empty() => arr,
            _ => return LoadResult::Empty {},
        };

        match self.parse_track(&tracks[0]) {
            Some(track) => LoadResult::Track(track),
            None => LoadResult::Empty {},
        }
    }

    async fn load_album(&self, seokey: &str) -> LoadResult {
        let params = [
            ("type", "albumDetail"),
            ("seokey", seokey),
        ];

        let data = match self.get_json(&params, &format!("album/{}", seokey)).await {
            Some(d) => d,
            None => return LoadResult::Empty {},
        };

        let tracks_arr = match data.get("tracks").and_then(|v| v.as_array()) {
            Some(arr) if !arr.is_empty() => arr,
            _ => return LoadResult::Empty {},
        };

        let album = data.get("album").unwrap_or(&Value::Null);
        let name = album
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown Album");
        let artwork = album.get("atw").and_then(|v| v.as_str());
        let author = album
            .get("artist")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|a| a.get("name"))
            .and_then(|v| v.as_str());

        let tracks: Vec<Track> = tracks_arr
            .iter()
            .take(self.album_load_limit)
            .filter_map(|t| self.parse_track(t))
            .collect();

        if tracks.is_empty() {
            return LoadResult::Empty {};
        }

        LoadResult::Playlist(PlaylistData {
            info: PlaylistInfo {
                name: name.to_string(),
                selected_track: 0,
            },
            plugin_info: serde_json::json!({
                "type": "album",
                "url": format!("https://gaana.com/album/{}", seokey),
                "artwork": artwork,
                "author": author,
            }),
            tracks,
        })
    }

    async fn load_playlist(&self, seokey: &str) -> LoadResult {
        let params = [
            ("type", "playlistDetail"),
            ("seokey", seokey),
        ];

        let data = match self.get_json(&params, &format!("playlist/{}", seokey)).await {
            Some(d) => d,
            None => return LoadResult::Empty {},
        };

        let tracks_arr = match data.get("tracks").and_then(|v| v.as_array()) {
            Some(arr) if !arr.is_empty() => arr,
            _ => return LoadResult::Empty {},
        };

        let playlist = data.get("playlist").unwrap_or(&Value::Null);
        let name = playlist
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown Playlist");
        let artwork = playlist.get("atw").and_then(|v| v.as_str());

        let tracks: Vec<Track> = tracks_arr
            .iter()
            .take(self.playlist_load_limit)
            .filter_map(|t| self.parse_track(t))
            .collect();

        if tracks.is_empty() {
            return LoadResult::Empty {};
        }

        LoadResult::Playlist(PlaylistData {
            info: PlaylistInfo {
                name: name.to_string(),
                selected_track: 0,
            },
            plugin_info: serde_json::json!({
                "type": "playlist",
                "url": format!("https://gaana.com/playlist/{}", seokey),
                "artwork": artwork,
            }),
            tracks,
        })
    }

    async fn load_artist(&self, seokey: &str) -> LoadResult {
        // Step 1: Get artist detail for artist_id
        let detail_params = [
            ("type", "artistDetail"),
            ("seokey", seokey),
        ];

        let detail = match self.get_json(&detail_params, &format!("artist/{}", seokey)).await {
            Some(d) => d,
            None => return LoadResult::Empty {},
        };

        let artist_arr = match detail.get("artist").and_then(|v| v.as_array()) {
            Some(arr) if !arr.is_empty() => arr,
            _ => return LoadResult::Empty {},
        };

        let artist_data = &artist_arr[0];
        let artist_id = match artist_data.get("artist_id").and_then(|v| {
            v.as_str()
                .map(|s| s.to_string())
                .or_else(|| v.as_i64().map(|i| i.to_string()))
        }) {
            Some(id) => id,
            None => return LoadResult::Empty {},
        };

        let artist_name = artist_data
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown Artist");
        let artwork = artist_data.get("artwork_bio").and_then(|v| v.as_str());

        // Step 2: Get artist track list
        let tracks_params = [
            ("language", ""),
            ("order", "0"),
            ("page", "0"),
            ("sortBy", "popularity"),
            ("type", "artistTrackList"),
            ("id", &artist_id),
        ];

        let tracks_data =
            match self.get_json(&tracks_params, &format!("artist/{}", seokey)).await {
                Some(d) => d,
                None => return LoadResult::Empty {},
            };

        // artistTrackList returns entities, not tracks
        let entities_arr = match tracks_data.get("entities").and_then(|v| v.as_array()) {
            Some(arr) if !arr.is_empty() => arr,
            _ => return LoadResult::Empty {},
        };

        let tracks: Vec<Track> = entities_arr
            .iter()
            .take(self.artist_load_limit)
            .filter_map(|t| self.parse_entity_track(t))
            .collect();

        if tracks.is_empty() {
            return LoadResult::Empty {};
        }

        LoadResult::Playlist(PlaylistData {
            info: PlaylistInfo {
                name: format!("{}'s Top Tracks", artist_name),
                selected_track: 0,
            },
            plugin_info: serde_json::json!({
                "type": "artist",
                "url": format!("https://gaana.com/artist/{}", seokey),
                "artwork": artwork,
            }),
            tracks,
        })
    }

    async fn search(&self, query: &str) -> LoadResult {
        debug!("Gaana searching: {}", query);

        let params = [
            ("country", "IN"),
            ("page", "0"),
            ("secType", "track"),
            ("type", "search"),
            ("keyword", query),
        ];

        let data = match self.get_json(&params, &format!("search/{}", urlencoding::encode(query)))
            .await
        {
            Some(d) => d,
            None => {
                return LoadResult::Error(LoadError {
                    message: "Gaana search failed".to_string(),
                    severity: crate::common::Severity::Common,
                    cause: "".to_string(),
                })
            }
        };

        let gr = match data.get("gr").and_then(|v| v.as_array()) {
            Some(arr) => arr,
            None => return LoadResult::Empty {},
        };

        // Find the "Track" group
        let track_group = gr.iter().find(|g| {
            g.get("ty").and_then(|v| v.as_str()) == Some("Track")
        });

        let items = match track_group
            .and_then(|g| g.get("gd"))
            .and_then(|v| v.as_array())
        {
            Some(arr) if !arr.is_empty() => arr,
            _ => return LoadResult::Empty {},
        };

        // For search, we need to fetch full song details for each result
        let mut results = Vec::new();
        for item in items.iter().take(self.search_limit) {
            let seokey = item
                .get("seo")
                .and_then(|v| v.as_str())
                .or_else(|| item.get("id").and_then(|v| v.as_str()));

            if let Some(key) = seokey {
                if let LoadResult::Track(track) = self.load_song(key).await {
                    results.push(track);
                }
            }
        }

        if results.is_empty() {
            LoadResult::Empty {}
        } else {
            LoadResult::Search(results)
        }
    }

    fn parse_track(&self, json: &Value) -> Option<Track> {
        let id = json.get("track_id").and_then(|v| {
            v.as_str()
                .map(|s| s.to_string())
                .or_else(|| v.as_i64().map(|i| i.to_string()))
                .or_else(|| v.as_u64().map(|i| i.to_string()))
        });

        // Fallback to entity_id
        let id = id.or_else(|| {
            json.get("entity_id").and_then(|v| {
                v.as_str()
                    .map(|s| s.to_string())
                    .or_else(|| v.as_i64().map(|i| i.to_string()))
            })
        })?;

        let title = json
            .get("track_title")
            .and_then(|v| v.as_str())
            .or_else(|| json.get("name").and_then(|v| v.as_str()))?;

        // Duration in seconds → milliseconds
        let duration_secs = json
            .get("duration")
            .and_then(|v| {
                v.as_u64()
                    .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
            })
            .unwrap_or(0);
        let duration = duration_secs * 1000;

        // Artist - check "artist" array first, then fallback to "artist_detail" (filter Singers)
        let author = if let Some(artist_arr) = json.get("artist").and_then(|v| v.as_array()) {
            let names: Vec<&str> = artist_arr
                .iter()
                .filter_map(|a| a.get("name").and_then(|v| v.as_str()))
                .collect();
            if names.is_empty() {
                "Unknown Artist".to_string()
            } else {
                names.join(", ")
            }
        } else if let Some(detail_arr) = json.get("artist_detail").and_then(|v| v.as_array()) {
            // Per formatJsonSongDetails: filter by role containing "Singer"
            let singers: Vec<&str> = detail_arr
                .iter()
                .filter(|a| {
                    a.get("role")
                        .and_then(|r| r.as_str())
                        .map(|r| r.contains("Singer"))
                        .unwrap_or(false)
                })
                .filter_map(|a| a.get("name").and_then(|v| v.as_str()))
                .collect();
            if singers.is_empty() {
                // Fallback: use all artist_detail names
                detail_arr
                    .iter()
                    .filter_map(|a| a.get("name").and_then(|v| v.as_str()))
                    .collect::<Vec<_>>()
                    .join(", ")
            } else {
                singers.join(", ")
            }
        } else {
            "Unknown Artist".to_string()
        };

        let seokey = json.get("seokey").and_then(|v| v.as_str());
        let uri = seokey.map(|s| format!("https://gaana.com/song/{}", s));

        let artwork_url = json
            .get("artwork_large")
            .and_then(|v| v.as_str())
            .or_else(|| json.get("atw").and_then(|v| v.as_str()))
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
            length: duration,
            is_stream: false,
            position: 0,
            title: title.to_string(),
            uri,
            artwork_url,
            isrc,
            source_name: "gaana".to_string(),
        };

        Some(Track::new(track_info))
    }

    /// Parse entity-format tracks (used in artist track lists)
    fn parse_entity_track(&self, json: &Value) -> Option<Track> {
        let id = json.get("entity_id").and_then(|v| {
            v.as_str()
                .map(|s| s.to_string())
                .or_else(|| v.as_i64().map(|i| i.to_string()))
        })?;

        let title = json.get("name").and_then(|v| v.as_str())?;

        let entity_info = json.get("entity_info").and_then(|v| v.as_array());

        let get_entity_value = |key: &str| -> Option<&Value> {
            entity_info?.iter().find_map(|e| {
                if e.get("key").and_then(|k| k.as_str()) == Some(key) {
                    e.get("value")
                } else {
                    None
                }
            })
        };

        let duration_secs = get_entity_value("duration")
            .and_then(|v| {
                v.as_u64()
                    .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
            })
            .unwrap_or(0);
        let duration = duration_secs * 1000;

        let author = get_entity_value("artist")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|a| a.get("name").and_then(|n| n.as_str()))
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_else(|| "Unknown Artist".to_string());

        let seokey = json.get("seokey").and_then(|v| v.as_str());
        let uri = seokey.map(|s| format!("https://gaana.com/song/{}", s));

        let artwork_url = json.get("atw").and_then(|v| v.as_str()).map(|s| s.to_string());

        let isrc = get_entity_value("isrc")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        let track_info = TrackInfo {
            identifier: id,
            is_seekable: true,
            author,
            length: duration,
            is_stream: false,
            position: 0,
            title: title.to_string(),
            uri,
            artwork_url,
            isrc,
            source_name: "gaana".to_string(),
        };

        Some(Track::new(track_info))
    }

    // ───────────── Playback URL ─────────────

    /// Fetch the stream URL from Gaana's stream API and decrypt it.
    /// Returns the decrypted HLS URL (.m3u8) which the audio pipeline routes
    /// through `HlsReader` for segment-based playback with seek support.
    async fn fetch_stream_url(&self, track_id: &str) -> Option<String> {
        let body = format!(
            "quality={}&track_id={}&stream_format=mp4",
            urlencoding::encode(&self.stream_quality),
            urlencoding::encode(track_id)
        );

        let resp = match self
            .client
            .post(STREAM_URL_API)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                warn!("Gaana: stream URL request failed: {}", e);
                return None;
            }
        };

        if !resp.status().is_success() {
            warn!("Gaana: stream URL API returned {}", resp.status());
            return None;
        }

        let data: Value = match resp.json().await {
            Ok(d) => d,
            Err(e) => {
                warn!("Gaana: failed to parse stream URL response: {}", e);
                return None;
            }
        };

        if data.get("api_status").and_then(|v| v.as_str()) != Some("success") {
            warn!("Gaana: stream API returned non-success status");
            return None;
        }

        let encrypted_path = data
            .get("data")
            .and_then(|d| d.get("stream_path"))
            .and_then(|v| v.as_str())?;

        let hls_url = crypto::decrypt_stream_path(encrypted_path)?;
        debug!("Gaana: decrypted HLS URL: {}", hls_url);

        Some(hls_url)
    }
}

#[async_trait]
impl SourcePlugin for GaanaSource {
    fn name(&self) -> &str {
        "gaana"
    }

    fn can_handle(&self, identifier: &str) -> bool {
        identifier.starts_with(&self.search_prefix)
            || identifier.starts_with("gaanasearch:")
            || self.url_regex.is_match(identifier)
    }

    async fn load(
        &self,
        identifier: &str,
        _routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> LoadResult {
        // Search
        if identifier.starts_with(&self.search_prefix) {
            let query = identifier.strip_prefix(&self.search_prefix).unwrap().trim();
            return self.search(query).await;
        }
        if identifier.starts_with("gaanasearch:") {
            let query = identifier.strip_prefix("gaanasearch:").unwrap().trim();
            return self.search(query).await;
        }

        // URL matching
        if let Some(caps) = self.url_regex.captures(identifier) {
            let type_ = caps.name("type").map(|m| m.as_str()).unwrap_or("");
            let seokey = caps.name("seokey").map(|m| m.as_str()).unwrap_or("");

            if seokey.is_empty() || type_.is_empty() {
                return LoadResult::Empty {};
            }

            return match type_ {
                "song" => self.load_song(seokey).await,
                "album" => self.load_album(seokey).await,
                "playlist" => self.load_playlist(seokey).await,
                "artist" => self.load_artist(seokey).await,
                _ => LoadResult::Empty {},
            };
        }

        LoadResult::Empty {}
    }

    async fn get_playback_url(
        &self,
        identifier: &str,
        _routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<String> {
        // Extract track_id from the identifier or URL
        let track_id = if let Some(caps) = self.url_regex.captures(identifier) {
            let type_ = caps.name("type").map(|m| m.as_str()).unwrap_or("");
            let seokey = caps.name("seokey").map(|m| m.as_str()).unwrap_or("");

            if type_ != "song" || seokey.is_empty() {
                return None;
            }

            // Need to fetch the track_id from the song detail
            let params = [
                ("type", "songDetail"),
                ("seokey", seokey),
            ];

            let data = self.get_json(&params, &format!("song/{}", seokey)).await?;
            data.get("tracks")
                .and_then(|v| v.as_array())
                .and_then(|arr| arr.first())
                .and_then(|t| {
                    t.get("track_id").and_then(|v| {
                        v.as_str()
                            .map(|s| s.to_string())
                            .or_else(|| v.as_i64().map(|i| i.to_string()))
                            .or_else(|| v.as_u64().map(|i| i.to_string()))
                    })
                })?
        } else {
            // Assume the identifier IS the track_id (numeric) or seokey
            if identifier.chars().all(|c| c.is_ascii_digit()) {
                identifier.to_string()
            } else {
                // Try to fetch track_id from songDetail
                let params = [
                    ("type", "songDetail"),
                    ("seokey", identifier),
                ];

                let data = self.get_json(&params, &format!("song/{}", identifier)).await?;
                data.get("tracks")
                    .and_then(|v| v.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|t| {
                        t.get("track_id").and_then(|v| {
                            v.as_str()
                                .map(|s| s.to_string())
                                .or_else(|| v.as_i64().map(|i| i.to_string()))
                                .or_else(|| v.as_u64().map(|i| i.to_string()))
                        })
                    })?
            }
        };

        self.fetch_stream_url(&track_id).await
    }

    fn get_proxy_config(&self) -> Option<crate::configs::HttpProxyConfig> {
        self.proxy.clone()
    }
}
