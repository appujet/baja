use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use regex::Regex;
use serde_json::Value;
use tracing::{debug, error, warn, trace};

use crate::{
    api::tracks::{LoadResult, PlaylistData, PlaylistInfo, Track, TrackInfo},
    sources::{SourcePlugin, plugin::PlayableTrack},
};

use super::track::{SoundCloudStreamKind, SoundCloudTrack};
use super::token::SoundCloudTokenTracker;

const BASE_URL: &str = "https://api-v2.soundcloud.com";

/// SoundCloud audio source.
pub struct SoundCloudSource {
    client: reqwest::Client,
    config: crate::configs::SoundCloudConfig,
    token_tracker: Arc<SoundCloudTokenTracker>,
    /// Regex patterns
    track_url_re: Regex,
    playlist_url_re: Regex,
    liked_url_re: Regex,
    short_url_re: Regex,
    mobile_url_re: Regex,
    liked_user_urn_re: Regex,
}

impl SoundCloudSource {
    pub fn new(config: crate::configs::SoundCloudConfig) -> Self {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36"
                .parse()
                .unwrap(),
        );

        let mut client_builder = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(15));

        if let Some(proxy_cfg) = &config.proxy {
            if let Some(url) = &proxy_cfg.url {
                debug!("Configuring proxy for SoundCloudSource: {}", url);
                if let Ok(mut proxy) = reqwest::Proxy::all(url) {
                    if let (Some(u), Some(p)) = (&proxy_cfg.username, &proxy_cfg.password) {
                        proxy = proxy.basic_auth(u, p);
                    }
                    client_builder = client_builder.proxy(proxy);
                }
            }
        }

        let client = client_builder.build().expect("Failed to build reqwest client");

        let token_tracker = Arc::new(SoundCloudTokenTracker::new(client.clone(), &config));
        token_tracker.clone().init();

        Self {
            client,
            config,
            token_tracker,
            track_url_re: Regex::new(
                r"^https?://(?:www\.|m\.)?soundcloud\.com/([a-zA-Z0-9_-]+)/([a-zA-Z0-9_-]+)(?:/s-[a-zA-Z0-9_-]+)?/?(?:\?.*)?$"
            ).unwrap(),
            playlist_url_re: Regex::new(
                r"^https?://(?:www\.|m\.)?soundcloud\.com/([a-zA-Z0-9_-]+)/sets/([a-zA-Z0-9_:-]+)(?:/[a-zA-Z0-9_-]+)?/?(?:\?.*)?$"
            ).unwrap(),
            liked_url_re: Regex::new(
                r"^https?://(?:www\.|m\.)?soundcloud\.com/([a-zA-Z0-9_-]+)/likes/?(?:\?.*)?$"
            ).unwrap(),
            short_url_re: Regex::new(
                r"^https://on\.soundcloud\.com/[a-zA-Z0-9_-]+/?(?:\?.*)?$"
            ).unwrap(),
            mobile_url_re: Regex::new(
                r"^https://soundcloud\.app\.goo\.gl/[a-zA-Z0-9_-]+/?(?:\?.*)?$"
            ).unwrap(),
            liked_user_urn_re: Regex::new(
                r#""urn":"soundcloud:users:(\d+)","username":"([^"]+)""#
            ).unwrap(),
        }
    }

    /// Resolve a URL via the SoundCloud resolve API.
    async fn api_resolve(&self, url: &str, client_id: &str) -> Option<Value> {
        let req_url = format!(
            "{}/resolve?url={}&client_id={}",
            BASE_URL,
            urlencoding::encode(url),
            client_id
        );
        debug!("SoundCloud: Resolving URL: {}", req_url);
        
        let builder = self.client.get(&req_url);

        let resp = builder.send().await.ok()?;
        if resp.status().as_u16() == 401 {
            self.token_tracker.invalidate().await;
            return None;
        }
        if !resp.status().is_success() {
            warn!("SoundCloud: API resolve failed with status: {} for {}", resp.status(), url);
            return None;
        }
        let json: Value = resp.json().await.ok()?;
        trace!("SoundCloud: API resolve response: {:?}", json);
        Some(json)
    }

    /// Build a Track from a SC API track JSON object.
    fn parse_track(&self, json: &Value) -> Result<Track, String> {
        let id = json.get("id").map(|v| v.to_string().trim_matches('"').to_string())
            .ok_or_else(|| "Missing track ID".to_string())?;

        let title = json
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown")
            .to_string();
        
        trace!("SoundCloud: Parsing track {}: {}", id, title);

        // Check if track is blocked
        if json.get("policy").and_then(|v| v.as_str()) == Some("BLOCK") {
            trace!("SoundCloud: Track '{}' is blocked by policy (likely geo-blocked). Returning metadata for mirroring.", title);
        }

        if json.get("monetization_model").and_then(|v| v.as_str()) == Some("SUB_HIGH_TIER") {
            trace!("SoundCloud: Track '{}' is a Go+ (premium) track", title);
        }

        // Check for preview-only tracks
        if let Some(transcodings) = json.get("media").and_then(|m| m.get("transcodings")).and_then(|v| v.as_array()) {
            let all_preview = !transcodings.is_empty() && transcodings.iter().all(|t| {
                let snipped = t.get("snipped").and_then(|v| v.as_bool()).unwrap_or(false);
                let url = t.get("url").and_then(|v| v.as_str()).unwrap_or("");
                snipped || url.contains("/preview/") || url.contains("cf-preview-media.sndcdn.com")
            });
            if all_preview {
                trace!("SoundCloud: Track '{}' only has preview transcodings. Returning metadata for mirroring.", title);
            }
        }

        let author = json
            .get("user")
            .and_then(|u| u.get("username"))
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown")
            .to_string();
        let duration = json
            .get("full_duration")
            .or_else(|| json.get("duration"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let uri = json
            .get("permalink_url")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let artwork_url = json
            .get("artwork_url")
            .and_then(|v| v.as_str())
            .map(|s| s.replace("-large", "-t500x500"));
        let isrc = json
            .get("publisher_metadata")
            .and_then(|m| m.get("isrc"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        Ok(Track::new(TrackInfo {
            identifier: id,
            is_seekable: true,
            author,
            length: duration,
            is_stream: false,
            position: 0,
            title,
            uri,
            artwork_url,
            isrc,
            source_name: "soundcloud".to_string(),
        }))
    }

    /// Select the best transcoding format and return (stream_kind, lookup_url).
    fn select_format(transcodings: &[Value]) -> Option<(SoundCloudStreamKind, String)> {
        if transcodings.is_empty() {
            return None;
        }

        macro_rules! find_transcoding {
            ($protocol:expr, $mime_contains:expr) => {
                transcodings.iter().find(|t| {
                    let fmt = t.get("format");
                    let proto = fmt
                        .and_then(|f| f.get("protocol"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let mime = fmt
                        .and_then(|f| f.get("mime_type"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let snipped = t.get("snipped").and_then(|v| v.as_bool()).unwrap_or(false);
                    let url = t.get("url").and_then(|v| v.as_str()).unwrap_or("");
                    
                    !snipped && !url.contains("/preview/") && !url.contains("cf-preview-media.sndcdn.com") 
                        && proto == $protocol && mime.contains($mime_contains)
                })
            };
        }

        // Priority: progressive mp3 > progressive aac > hls opus > hls mp3 > hls aac > any progressive > any hls
        let selected = find_transcoding!("progressive", "mpeg")
            .or_else(|| find_transcoding!("progressive", "aac"))
            .or_else(|| find_transcoding!("hls", "mpeg"))
            .or_else(|| find_transcoding!("hls", "aac"))
            .or_else(|| find_transcoding!("hls", "mp4"))
            .or_else(|| find_transcoding!("hls", "m4a"))
            .or_else(|| find_transcoding!("hls", "ogg"))
            .or_else(|| {
                transcodings
                    .iter()
                    .find(|t| {
                        t.get("format")
                            .and_then(|f| f.get("protocol"))
                            .and_then(|v| v.as_str())
                            == Some("progressive")
                    })
            })
            .or_else(|| {
                transcodings.iter().find(|t| {
                    t.get("format")
                        .and_then(|f| f.get("protocol"))
                        .and_then(|v| v.as_str())
                        == Some("hls")
                })
            })
            .or_else(|| transcodings.first())?;

        let lookup_url = selected.get("url").and_then(|v| v.as_str())?.to_string();
        let proto = selected
            .get("format")
            .and_then(|f| f.get("protocol"))
            .and_then(|v| v.as_str())
            .unwrap_or("progressive");
        let mime = selected
            .get("format")
            .and_then(|f| f.get("mime_type"))
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let kind = if proto == "progressive" {
            if mime.contains("mpeg") || mime.contains("mp3") {
                SoundCloudStreamKind::ProgressiveMp3
            } else {
                SoundCloudStreamKind::ProgressiveAac
            }
        } else {
            // HLS
            if mime.contains("ogg") {
                SoundCloudStreamKind::HlsOpus
            } else if mime.contains("mpeg") || mime.contains("mp3") {
                SoundCloudStreamKind::HlsMp3
            } else if mime.contains("aac") || mime.contains("mp4") || mime.contains("m4a") {
                SoundCloudStreamKind::HlsAac
            } else {
                // Unknown HLS format, try as AAC/TS
                SoundCloudStreamKind::HlsAac
            }
        };

        Some((kind, lookup_url))
    }

    /// Resolve the actual stream URL from a transcoding lookup URL.
    async fn resolve_stream_url(&self, lookup_url: &str, client_id: &str) -> Option<String> {
        let url = format!("{}?client_id={}", lookup_url, client_id);
        let builder = self.client.get(&url);

        let resp = builder.send().await.ok()?;
        if resp.status().as_u16() == 401 {
            self.token_tracker.invalidate().await;
            return None;
        }
        if !resp.status().is_success() {
            return None;
        }
        let json: Value = resp.json().await.ok()?;
        let stream_url = json.get("url").and_then(|v| v.as_str()).map(|s| s.to_string());
        if let Some(ref url) = stream_url {
            debug!("SoundCloud: Resolved playback URL: {}", url);
        }
        stream_url
    }

    /// Resolve a track URL and return a PlayableTrack.
    async fn get_track_from_url(
        &self,
        url: &str,
        client_id: &str,
        local_addr: Option<std::net::IpAddr>,
    ) -> Option<Box<dyn PlayableTrack>> {
        let json = self.api_resolve(url, client_id).await?;

        if json.get("kind").and_then(|v| v.as_str()) != Some("track") {
            return None;
        }

        let transcodings = json
            .get("media")
            .and_then(|m| m.get("transcodings"))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        if transcodings.is_empty() {
            warn!("SoundCloud: No transcodings for track {}", url);
            return None;
        }

        let (kind, lookup_url) = Self::select_format(&transcodings)?;
        trace!("SoundCloud: Selected format {:?} for {}", kind, url);

        let stream_url = self.resolve_stream_url(&lookup_url, client_id).await?;

        // Filter preview URLs
        if stream_url.contains("cf-preview-media.sndcdn.com") || stream_url.contains("/preview/") {
            warn!("SoundCloud: Track {} only has a preview URL, skipping", url);
            return None;
        }

        Some(Box::new(SoundCloudTrack {
            stream_url,
            kind,
            local_addr,
            proxy: self.config.proxy.clone(),
        }))
    }

    async fn search_tracks(&self, query: &str) -> LoadResult {
        let client_id = match self.token_tracker.get_client_id().await {
            Some(id) => id,
            None => return LoadResult::Empty {},
        };

        let limit = self.config.search_limit;
        let req_url = format!(
            "{}/search/tracks?q={}&client_id={}&limit={}&offset=0",
            BASE_URL,
            urlencoding::encode(query),
            client_id,
            limit
        );

        let builder = self.client.get(&req_url);

        let resp = match builder.send().await {
            Ok(r) => r,
            Err(e) => {
                error!("SoundCloud search error: {}", e);
                return LoadResult::Empty {};
            }
        };

        if !resp.status().is_success() {
            return LoadResult::Empty {};
        }

        let json: Value = match resp.json().await {
            Ok(v) => v,
            Err(_) => return LoadResult::Empty {},
        };

        let tracks: Vec<Track> = json
            .get("collection")
            .and_then(|v| v.as_array())
            .unwrap_or(&vec![])
            .iter()
            .filter_map(|item| self.parse_track(item).ok())
            .collect();

        if tracks.is_empty() {
            LoadResult::Empty {}
        } else {
            LoadResult::Search(tracks)
        }
    }

    async fn load_single_track(&self, url: &str) -> LoadResult {
        let client_id = match self.token_tracker.get_client_id().await {
            Some(id) => id,
            None => return LoadResult::Empty {},
        };

        let json = match self.api_resolve(url, &client_id).await {
            Some(v) => v,
            None => return LoadResult::Empty {},
        };

        match self.parse_track(&json) {
            Ok(track) => LoadResult::Track(track),
            Err(msg) => {
                warn!("SoundCloud: Failed to parse track: {}", msg);
                LoadResult::Empty {}
            }
        }
    }

    async fn resolve_short_url(&self, url: &str) -> Option<String> {
        // Do a HEAD request with no redirects to get the Location header
        let resp = self
            .client
            .head(url)
            .send()
            .await
            .ok()?;

        let location = resp.headers().get("location")?.to_str().ok()?.to_string();
        Some(location)
    }

    async fn resolve_mobile_url(&self, url: &str) -> Option<String> {
        // Follow redirects and return final URL
        let resp = self.client.get(url).send().await.ok()?;
        Some(resp.url().to_string())
    }

    async fn load_playlist(&self, url: &str) -> LoadResult {
        let client_id = match self.token_tracker.get_client_id().await {
            Some(id) => id,
            None => return LoadResult::Empty {},
        };

        let json = match self.api_resolve(url, &client_id).await {
            Some(v) => v,
            None => return LoadResult::Empty {},
        };

        if json.get("kind").and_then(|v| v.as_str()) != Some("playlist") {
            return LoadResult::Empty {};
        }

        let name = json
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("Untitled playlist")
            .to_string();

        let raw_tracks = json
            .get("tracks")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        // Split into complete tracks (have title) and stub (only id)
        let mut complete: Vec<Track> = Vec::new();
        let mut stub_ids: Vec<String> = Vec::new();

        for t in &raw_tracks {
            if t.get("title").is_some() {
                if let Some(track) = self.parse_track(t).ok() {
                    complete.push(track);
                }
            } else if let Some(id) = t.get("id").map(|v| v.to_string()) {
                stub_ids.push(id);
            }
        }

        let playlist_limit = self.config.playlist_load_limit;
        let needed = stub_ids
            .iter()
            .take(playlist_limit.saturating_sub(complete.len()))
            .cloned()
            .collect::<Vec<_>>();

        // Batch fetch stub tracks in groups of 50
        for chunk in needed.chunks(50) {
            let ids = chunk.join(",");
            let batch_url = format!(
                "{}/tracks?ids={}&client_id={}",
                BASE_URL, ids, client_id
            );

            let builder = self.client.get(&batch_url);

            if let Ok(resp) = builder.send().await {
                if let Ok(json) = resp.json::<Value>().await {
                    if let Some(arr) = json.as_array() {
                        for item in arr {
                            if let Some(track) = self.parse_track(item).ok() {
                                complete.push(track);
                            }
                        }
                    }
                }
            }
        }

        // Respect playlist limit
        complete.truncate(playlist_limit);

        if complete.is_empty() {
            return LoadResult::Empty {};
        }

        LoadResult::Playlist(PlaylistData {
            info: PlaylistInfo {
                name,
                selected_track: -1,
            },
            plugin_info: Value::Null,
            tracks: complete,
        })
    }

    async fn load_liked_tracks(&self, url: &str) -> LoadResult {
        let client_id = match self.token_tracker.get_client_id().await {
            Some(id) => id,
            None => return LoadResult::Empty {},
        };

        // Fetch the liked page HTML to extract user ID
        let html = match self.client.get(url).send().await {
            Ok(r) => match r.text().await {
                Ok(t) => t,
                Err(_) => return LoadResult::Empty {},
            },
            Err(_) => return LoadResult::Empty {},
        };

        let caps = match self.liked_user_urn_re.captures(&html) {
            Some(c) => c,
            None => return LoadResult::Empty {},
        };

        let user_id = caps.get(1).map(|m| m.as_str()).unwrap_or("");
        let user_name = caps.get(2).map(|m| m.as_str()).unwrap_or("User");

        let liked_url = format!(
            "{}/users/{}/likes?limit=200&offset=0&client_id={}",
            BASE_URL, user_id, client_id
        );

        let resp = match self.client.get(&liked_url).send().await {
            Ok(r) => r,
            Err(_) => return LoadResult::Empty {},
        };

        let json: Value = match resp.json().await {
            Ok(v) => v,
            Err(_) => return LoadResult::Empty {},
        };

        let tracks: Vec<Track> = json
            .get("collection")
            .and_then(|v| v.as_array())
            .unwrap_or(&vec![])
            .iter()
            .filter_map(|item| {
                // Liked items have a "track" sub-object
                item.get("track").and_then(|t| self.parse_track(t).ok())
            })
            .collect();

        if tracks.is_empty() {
            return LoadResult::Empty {};
        }

        LoadResult::Playlist(PlaylistData {
            info: PlaylistInfo {
                name: format!("Liked by {}", user_name),
                selected_track: -1,
            },
            plugin_info: Value::Null,
            tracks,
        })
    }
}

#[async_trait]
impl SourcePlugin for SoundCloudSource {
    fn name(&self) -> &str {
        "soundcloud"
    }

    fn can_handle(&self, identifier: &str) -> bool {
        if identifier.starts_with("scsearch:") {
            return true;
        }
        // Normalize: strip mobile prefix
        let url = identifier
            .strip_prefix("https://m.")
            .map(|s| format!("https://{}", s))
            .unwrap_or_else(|| identifier.to_string());

        self.short_url_re.is_match(&url)
            || self.mobile_url_re.is_match(identifier)
            || self.liked_url_re.is_match(&url)
            || self.playlist_url_re.is_match(&url)
            || self.track_url_re.is_match(&url)
    }

    async fn load(
        &self,
        identifier: &str,
        _routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> LoadResult {
        // 1. Search
        if let Some(query) = identifier.strip_prefix("scsearch:") {
            return self.search_tracks(query.trim()).await;
        }

        // 2. Resolve redirects
        let url = if self.mobile_url_re.is_match(identifier) {
            match self.resolve_mobile_url(identifier).await {
                Some(u) => u,
                None => return LoadResult::Empty {},
            }
        } else if self.short_url_re.is_match(identifier) {
            match self.resolve_short_url(identifier).await {
                Some(u) => u,
                None => return LoadResult::Empty {},
            }
        } else {
            // Strip mobile subdomain
            identifier
                .strip_prefix("https://m.")
                .map(|s| format!("https://{}", s))
                .unwrap_or_else(|| identifier.to_string())
        };

        // 3. Dispatch
        if self.liked_url_re.is_match(&url) {
            return self.load_liked_tracks(&url).await;
        }

        if self.playlist_url_re.is_match(&url) {
            return self.load_playlist(&url).await;
        }

        if self.track_url_re.is_match(&url) {
            return self.load_single_track(&url).await;
        }

        LoadResult::Empty {}
    }

    async fn get_track(
        &self,
        identifier: &str,
        routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<Box<dyn PlayableTrack>> {
        // Resolve identifier to a URL if needed
        let url = if self.mobile_url_re.is_match(identifier) {
            self.resolve_mobile_url(identifier).await?
        } else if self.short_url_re.is_match(identifier) {
            self.resolve_short_url(identifier).await?
        } else {
            identifier
                .strip_prefix("https://m.")
                .map(|s| format!("https://{}", s))
                .unwrap_or_else(|| identifier.to_string())
        };

        let client_id = self.token_tracker.get_client_id().await?;
        let local_addr = routeplanner.and_then(|rp| rp.get_address());

        self.get_track_from_url(&url, &client_id, local_addr).await
    }

    fn get_proxy_config(&self) -> Option<crate::configs::HttpProxyConfig> {
        self.config.proxy.clone()
    }
}
