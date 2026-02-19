use crate::api::tracks::{LoadResult, PlaylistData, PlaylistInfo, Track, TrackInfo};
use crate::sources::SourcePlugin;
use async_trait::async_trait;
use regex::Regex;
use reqwest::header::HeaderMap;

use serde_json::Value;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tracing::{debug, error, warn};

const PUBLIC_API_BASE: &str = "https://api.deezer.com/2.0";
const PRIVATE_API_BASE: &str = "https://www.deezer.com/ajax/gw-light.php";

#[derive(Debug, Clone)]
struct DeezerTokens {
    session_id: String,
    dzr_uniq_id: String,
    api_token: String,
    license_token: String,
    expire_at: Instant,
    arl_index: usize,
}

struct DeezerTokenTracker {
    client: reqwest::Client,
    arls: Vec<String>,
    tokens: Arc<Mutex<Vec<Option<DeezerTokens>>>>,
    current_index: AtomicUsize,
}

impl DeezerTokenTracker {
    fn new(client: reqwest::Client, arls: Vec<String>) -> Self {
        let size = arls.len();
        Self {
            client,
            arls,
            tokens: Arc::new(Mutex::new(vec![None; size])),
            current_index: AtomicUsize::new(0),
        }
    }

    async fn get_token(&self) -> Option<DeezerTokens> {
        let index = self.current_index.fetch_add(1, Ordering::Relaxed) % self.arls.len();
        self.get_token_at(index).await
    }

    async fn get_token_at(&self, index: usize) -> Option<DeezerTokens> {
        // Check if we have a valid cached token for this index
        {
            let guard = self.tokens.lock().unwrap();
            if let Some(tokens) = &guard[index] {
                if Instant::now() < tokens.expire_at {
                    return Some(tokens.clone());
                }
            }
        }

        // Needs refresh
        self.refresh_session(index).await
    }

    // Invalidate a specific token (e.g. if API rejected it despite being fresh time-wise)
    fn invalidate_token(&self, index: usize) {
        let mut guard = self.tokens.lock().unwrap();
        guard[index] = None;
    }

    async fn refresh_session(&self, index: usize) -> Option<DeezerTokens> {
        let arl = &self.arls[index];
        let initial_cookie = format!("arl={}", arl);

        let url = "https://www.deezer.com/ajax/gw-light.php?method=deezer.getUserData&input=3&api_version=1.0&api_token=";

        let req = self.client.get(url).header("Cookie", &initial_cookie);

        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                error!(
                    "Failed to refresh Deezer session for ARL index {}: {}",
                    index, e
                );
                return None;
            }
        };

        // Extract cookies from response
        let mut session_id = String::new();
        let mut dzr_uniq_id = String::new();

        for cookie in resp.cookies() {
            match cookie.name() {
                "sid" => session_id = cookie.value().to_string(),
                "dzr_uniq_id" => dzr_uniq_id = cookie.value().to_string(),
                _ => {}
            }
        }

        if session_id.is_empty() {
            warn!(
                "Failed to find sid cookie in response for ARL index {}",
                index
            );
        }
        if dzr_uniq_id.is_empty() {
            warn!(
                "Failed to find dzr_uniq_id cookie in response for ARL index {}",
                index
            );
        }

        let body: Value = match resp.json().await {
            Ok(v) => v,
            Err(e) => {
                error!("Failed to parse Deezer session response: {}", e);
                return None;
            }
        };

        let api_token = body
            .get("results")
            .and_then(|r| r.get("checkForm"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())?;

        let license_token = body
            .get("results")
            .and_then(|r| r.get("USER"))
            .and_then(|u| u.get("OPTIONS"))
            .and_then(|o| o.get("license_token"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_default();

        let tokens = DeezerTokens {
            session_id,
            dzr_uniq_id,
            api_token,
            license_token,
            expire_at: Instant::now() + Duration::from_secs(3600),
            arl_index: index,
        };

        {
            let mut guard = self.tokens.lock().unwrap();
            guard[index] = Some(tokens.clone());
        }

        debug!("Refreshed Deezer tokens for ARL index {}", index);
        Some(tokens)
    }
}

pub struct DeezerSource {
    client: reqwest::Client,
    config: crate::configs::DeezerConfig,
    token_tracker: DeezerTokenTracker,
    url_regex: Regex,
    search_prefix: String,
    isrc_prefix: String,
    rec_prefix: String,
    rec_artist_prefix: String,
    rec_track_prefix: String,
    share_url_prefix: String,
}

impl DeezerSource {
    pub fn new(config: crate::configs::DeezerConfig) -> Result<Self, String> {
        if config
            .master_decryption_key
            .as_deref()
            .unwrap_or("")
            .is_empty()
        {
            return Err("Deezer master_decryption_key must be set".to_string());
        }

        // Use arls directly, filtering out empties
        let mut arls = config.arls.clone().unwrap_or_default();
        arls.retain(|s| !s.is_empty());

        // Deduplicate
        arls.sort();
        arls.dedup();

        if arls.is_empty() {
            return Err("Deezer arls must be set and contain at least one valid token".to_string());
        }

        let mut headers = HeaderMap::new();
        headers.insert(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/115.0.0.0 Safari/537.36"
                .parse()
                .unwrap(),
        );

        let mut client_builder = reqwest::Client::builder()
            .default_headers(headers)
            .cookie_store(true);
        //.redirect(reqwest::redirect::Policy::none()); // Reverted

        if let Some(proxy_config) = &config.proxy {
            if let Some(url) = &proxy_config.url {
                debug!("Configuring proxy for DeezerSource: {}", url);
                if let Ok(mut proxy_obj) = reqwest::Proxy::all(url) {
                    // Basic auth if needed
                    if let (Some(username), Some(password)) =
                        (&proxy_config.username, &proxy_config.password)
                    {
                        proxy_obj = proxy_obj.basic_auth(username, password);
                    }
                    client_builder = client_builder.proxy(proxy_obj);
                }
            }
        }

        let client = client_builder
            .build()
            .map_err(|e| format!("Failed to build HTTP client: {}", e))?;
        let token_tracker = DeezerTokenTracker::new(client.clone(), arls);

        Ok(Self {
            client,
            config,
            token_tracker,
            url_regex: Regex::new(r"https?://(?:www\.)?deezer\.com/(?:[a-z]+(?:-[a-z]+)?/)?(?<type>track|album|playlist|artist)/(?<id>\d+)").unwrap(),
            search_prefix: "dzsearch:".to_string(),
            isrc_prefix: "dzisrc:".to_string(),
            rec_prefix: "dzrec:".to_string(),
            rec_artist_prefix: "artist=".to_string(),
            rec_track_prefix: "track=".to_string(),
            share_url_prefix: "https://deezer.page.link/".to_string(),
        })
    }

    async fn get_json_public(&self, path: &str) -> Option<Value> {
        let url = format!("{}/{}", PUBLIC_API_BASE, path);
        match self.client.get(&url).send().await {
            Ok(res) => res.json().await.ok(),
            Err(_) => None,
        }
    }

    fn parse_track(&self, json: &Value) -> Option<Track> {
        let id = json.get("id").map(|v| v.to_string())?;
        let title = json.get("title").and_then(|v| v.as_str())?.to_string();
        let artist = json
            .get("artist")
            .and_then(|a| a.get("name"))
            .and_then(|v| v.as_str())?
            .to_string();
        let duration = json.get("duration").and_then(|v| v.as_u64()).unwrap_or(0) * 1000;
        let isrc = json
            .get("isrc")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let artwork_url = json
            .get("album")
            .and_then(|a| a.get("cover_xl"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let uri = json
            .get("link")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let info = TrackInfo {
            identifier: id,
            is_seekable: true,
            author: artist,
            length: duration,
            is_stream: false,
            position: 0,
            title,
            uri,
            artwork_url,
            isrc,
            source_name: "deezer".to_string(),
        };

        Some(Track::new(info))
    }

    async fn search(&self, query: &str) -> LoadResult {
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

    async fn get_track_by_isrc(&self, isrc: &str) -> Option<Track> {
        let url = format!("track/isrc:{}", isrc);
        if let Some(json) = self.get_json_public(&url).await {
            if json.get("id").is_some() {
                return self.parse_track(&json);
            }
        }
        None
    }

    async fn get_recommendations(&self, query: &str) -> LoadResult {
        let tokens = match self.token_tracker.get_token().await {
            Some(t) => t,
            None => return LoadResult::Empty {},
        };

        let method;
        let payload;

        if let Some(artist_id) = query.strip_prefix(&self.rec_artist_prefix) {
            method = "song.getSmartRadio";
            payload = serde_json::json!({ "art_id": artist_id });
        } else {
            let track_id = if let Some(stripped) = query.strip_prefix(&self.rec_track_prefix) {
                stripped
            } else {
                query
            };
            method = "song.getSearchTrackMix";
            payload = serde_json::json!({ "sng_id": track_id, "start_with_input_track": "true" });
        }

        let url = format!(
            "{}?method={}&input=3&api_version=1.0&api_token={}",
            PRIVATE_API_BASE, method, tokens.api_token
        );

        let res = match self
            .client
            .post(&url)
            .header(
                "Cookie",
                format!(
                    "sid={}; dzr_uniq_id={}",
                    tokens.session_id, tokens.dzr_uniq_id
                ),
            )
            .json(&payload)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                debug!("Deezer: recommendations request failed: {}", e);
                return LoadResult::Empty {};
            }
        };

        let json: Value = match res.json().await {
            Ok(v) => v,
            Err(_) => return LoadResult::Empty {},
        };

        let tracks: Vec<Track> = if let Some(data) = json
            .get("results")
            .and_then(|r| r.get("data"))
            .and_then(|d| d.as_array())
        {
            data.iter()
                .filter_map(|item| self.parse_recommendation_track(item))
                .collect()
        } else if let Some(data) = json
            .get("results")
            .and_then(|r| r.get("data"))
            .and_then(|d| d.as_object())
        {
            // Sometimes it returns an object if empty or single? LavaSrc handles .values()
            data.values()
                .filter_map(|item| self.parse_recommendation_track(item))
                .collect()
        } else {
            Vec::new()
        };

        if tracks.is_empty() {
            return LoadResult::Empty {};
        }

        LoadResult::Playlist(PlaylistData {
            info: PlaylistInfo {
                name: "Deezer Recommendations".to_string(),
                selected_track: -1,
            },
            plugin_info: serde_json::Value::Null,
            tracks,
        })
    }

    async fn get_album(&self, id: &str) -> LoadResult {
        let json = match self.get_json_public(&format!("album/{}", id)).await {
            Some(j) => j,
            None => return LoadResult::Empty {},
        };

        if json
            .get("tracks")
            .and_then(|t| t.get("data"))
            .map(|d| d.as_array().map(|a| a.is_empty()).unwrap_or(true))
            .unwrap_or(true)
        {
            return LoadResult::Empty {};
        }

        let artwork_url = json
            .get("cover_xl")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let tracks_json = match self
            .get_json_public(&format!("album/{}/tracks?limit=10000", id))
            .await
        {
            Some(j) => j,
            None => return LoadResult::Empty {},
        };

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
                name: json
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown Album")
                    .to_string(),
                selected_track: -1,
            },
            plugin_info: serde_json::Value::Null,
            tracks,
        })
    }

    async fn get_playlist(&self, id: &str) -> LoadResult {
        let json = match self.get_json_public(&format!("playlist/{}", id)).await {
            Some(j) => j,
            None => return LoadResult::Empty {},
        };

        if json
            .get("tracks")
            .and_then(|t| t.get("data"))
            .map(|d| d.as_array().map(|a| a.is_empty()).unwrap_or(true))
            .unwrap_or(true)
        {
            return LoadResult::Empty {};
        }

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
            plugin_info: serde_json::Value::Null,
            tracks,
        })
    }

    async fn get_artist(&self, id: &str) -> LoadResult {
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

        if tracks_json
            .get("data")
            .and_then(|d| d.as_array())
            .map(|a| a.is_empty())
            .unwrap_or(true)
        {
            return LoadResult::Empty {};
        }

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
            plugin_info: serde_json::Value::Null,
            tracks,
        })
    }

    fn parse_recommendation_track(&self, json: &Value) -> Option<Track> {
        let id_str = if let Some(v) = json.get("SNG_ID") {
            if let Some(s) = v.as_str() {
                s.to_string()
            } else {
                v.to_string()
            }
        } else {
            return None;
        };

        let id = id_str;
        let title = json.get("SNG_TITLE").and_then(|v| v.as_str())?.to_string();
        let artist = json.get("ART_NAME").and_then(|v| v.as_str())?.to_string();
        let duration = json.get("DURATION").and_then(|v| v.as_u64()).unwrap_or(0) * 1000;
        let isrc = json
            .get("ISRC")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let album_pic = json
            .get("ALB_PICTURE")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let artwork_url = if !album_pic.is_empty() {
            Some(format!(
                "https://cdn-images.dzcdn.net/images/cover/{}/1000x1000-000000-80-0-0.jpg",
                album_pic
            ))
        } else {
            None
        };

        let uri = Some(format!("https://deezer.com/track/{}", id));

        let info = TrackInfo {
            identifier: id,
            is_seekable: true,
            author: artist,
            length: duration,
            is_stream: false,
            position: 0,
            title,
            uri,
            artwork_url,
            isrc,
            source_name: "deezer".to_string(),
        };

        Some(Track::new(info))
    }
}

#[async_trait]
impl SourcePlugin for DeezerSource {
    fn name(&self) -> &str {
        "deezer"
    }

    fn can_handle(&self, identifier: &str) -> bool {
        identifier.starts_with(&self.search_prefix)
            || identifier.starts_with(&self.isrc_prefix)
            || identifier.starts_with(&self.rec_prefix)
            || identifier.starts_with(&self.share_url_prefix)
            || self.url_regex.is_match(identifier)
    }

    async fn load(
        &self,
        identifier: &str,
        routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> LoadResult {
        if identifier.starts_with(&self.search_prefix) {
            let query = identifier.strip_prefix(&self.search_prefix).unwrap();
            return self.search(query).await;
        }

        if identifier.starts_with(&self.isrc_prefix) {
            let isrc = identifier.strip_prefix(&self.isrc_prefix).unwrap();
            if let Some(track) = self.get_track_by_isrc(isrc).await {
                return LoadResult::Track(track);
            }
            return LoadResult::Empty {};
        }

        if identifier.starts_with(&self.rec_prefix) {
            let query = identifier.strip_prefix(&self.rec_prefix).unwrap();
            return self.get_recommendations(query).await;
        }

        if identifier.starts_with(&self.share_url_prefix) {
            // Create a temporary client that doesn't follow redirects to get the location
            let client = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap_or_else(|_| self.client.clone());

            let res = match client.get(identifier).send().await {
                Ok(r) => r,
                Err(_) => return LoadResult::Empty {},
            };

            if res.status().is_redirection() {
                if let Some(location) = res.headers().get("location") {
                    if let Ok(loc_str) = location.to_str() {
                        if loc_str.starts_with("https://www.deezer.com/") {
                            // Recursively load the resolved URL
                            return self.load(loc_str, routeplanner).await;
                        }
                    }
                }
            }
            return LoadResult::Empty {};
        }

        if let Some(caps) = self.url_regex.captures(identifier) {
            let type_ = caps.name("type").map(|m| m.as_str()).unwrap_or("");
            let id = caps.name("id").map(|m| m.as_str()).unwrap_or("");

            match type_ {
                "track" => {
                    if let Some(json) = self.get_json_public(&format!("track/{}", id)).await {
                        if let Some(track) = self.parse_track(&json) {
                            return LoadResult::Track(track);
                        }
                    }
                }
                "album" => return self.get_album(id).await,
                "playlist" => return self.get_playlist(id).await,
                "artist" => return self.get_artist(id).await,
                _ => {}
            }
        }

        LoadResult::Empty {}
    }

    async fn get_playback_url(
        &self,
        identifier: &str,
        _routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<String> {
        let id_extracted = if let Some(caps) = self.url_regex.captures(identifier) {
            caps.name("id")
                .map(|m| m.as_str().to_string())
                .unwrap_or(identifier.to_string())
        } else {
            identifier.to_string()
        };

        debug!(
            "Deezer: Resolving playback URL for identifier: {} (ID: {})",
            identifier, id_extracted
        );

        let mut retry_count = 0;
        let max_retries = 3;

        loop {
            if retry_count > max_retries {
                debug!("Deezer: Max retries exceeded for {}", identifier);
                return None;
            }

            let tokens = match self.token_tracker.get_token().await {
                Some(t) => t,
                None => {
                    debug!("Deezer: Failed to get tokens (retry {})", retry_count);
                    retry_count += 1;
                    continue;
                }
            };

            // 1. Get Track Token
            let url = format!(
                "{}?method=song.getData&input=3&api_version=1.0&api_token={}",
                PRIVATE_API_BASE, tokens.api_token
            );
            let body = serde_json::json!({
                "sng_id": id_extracted
            });

            let res = match self
                .client
                .post(&url)
                .header(
                    "Cookie",
                    format!(
                        "sid={}; dzr_uniq_id={}",
                        tokens.session_id, tokens.dzr_uniq_id
                    ),
                )
                .json(&body)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    debug!("Deezer: song.getData request failed: {}", e);
                    retry_count += 1;
                    continue;
                }
            };

            let json: Value = match res.json().await {
                Ok(v) => v,
                Err(e) => {
                    debug!("Deezer: song.getData failed to parse JSON: {}", e);
                    retry_count += 1;
                    continue;
                }
            };

            if let Some(error) = json
                .get("error")
                .and_then(|v| v.as_array())
                .filter(|v| !v.is_empty())
            {
                debug!("Deezer: song.getData returned API error: {:?}", error);
                // If error indicates invalid token, we might want to invalidate and retry
                // For now we just loop, which gets next token
                self.token_tracker.invalidate_token(tokens.arl_index);
                retry_count += 1;
                continue;
            }

            let track_token = match json
                .get("results")
                .and_then(|r| r.get("TRACK_TOKEN"))
                .and_then(|v| v.as_str())
            {
                Some(t) => t,
                None => {
                    debug!("Deezer: TRACK_TOKEN not found in response: {}", json);
                    // This often means token is bad or track is restricted
                    self.token_tracker.invalidate_token(tokens.arl_index);
                    retry_count += 1;
                    continue;
                }
            };

            // 2. Get Media URL
            let media_url = "https://media.deezer.com/v1/get_url";
            let body = serde_json::json!({
                "license_token": tokens.license_token,
                "media": [{
                    "type": "FULL",
                    "formats": [
                        { "cipher": "BF_CBC_STRIPE", "format": "MP3_128" },
                        { "cipher": "BF_CBC_STRIPE", "format": "MP3_64" }
                    ]
                }],
                "track_tokens": [track_token]
            });

            let res = match self.client.post(media_url).json(&body).send().await {
                Ok(r) => r,
                Err(e) => {
                    debug!("Deezer: get_url request failed: {}", e);
                    retry_count += 1;
                    continue;
                }
            };

            let json: Value = match res.json().await {
                Ok(v) => v,
                Err(e) => {
                    debug!("Deezer: get_url failed to parse JSON: {}", e);
                    retry_count += 1;
                    continue;
                }
            };

            if let Some(errors) = json
                .get("data")
                .and_then(|d| d.get(0))
                .and_then(|d| d.get("errors"))
                .and_then(|e| e.as_array())
                .filter(|e| !e.is_empty())
            {
                debug!("Deezer: get_url returned errors: {:?}", errors);
                // Check for "License token..." error?
                // We just treat all errors as reason to retry with next token
                self.token_tracker.invalidate_token(tokens.arl_index);
                retry_count += 1;
                continue;
            }

            let url_opt = json
                .get("data")
                .and_then(|d| d.get(0))
                .and_then(|d| d.get("media"))
                .and_then(|m| m.get(0))
                .and_then(|m| m.get("sources"))
                .and_then(|s| s.get(0))
                .and_then(|s| s.get("url"))
                .and_then(|u| u.as_str());

            match url_opt {
                Some(url) => {
                    debug!("Deezer: Successfully resolved URL for {}", identifier);
                    return Some(format!("deezer_encrypted:{}:{}", id_extracted, url));
                }
                None => {
                    debug!(
                        "Deezer: Failed to extract media URL from response: {}",
                        json
                    );
                    self.token_tracker.invalidate_token(tokens.arl_index);
                    retry_count += 1;
                    continue;
                }
            }
        }
    }

    fn get_proxy_config(&self) -> Option<crate::configs::HttpProxyConfig> {
        self.config.proxy.clone()
    }
}
