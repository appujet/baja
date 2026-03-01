use std::sync::Arc;

use async_trait::async_trait;
use regex::Regex;
use serde::Serialize;
use serde_json::{Value, json};
use tokio::sync::RwLock;
use tracing::{debug, warn};

use crate::{
    common::types::SharedRw,
    configs::sources::YouTubeConfig,
    protocol::tracks::*,
    sources::{BoxedTrack, SourcePlugin, StreamInfo},
};

pub mod cipher;
pub mod clients;
pub mod extractor;
pub mod hls;
pub mod oauth;
pub mod reader;
pub mod sabr;
pub mod ua;
pub mod utils;

pub mod track;

use cipher::YouTubeCipherManager;
use clients::{
    YouTubeClient, android::AndroidClient, android_vr::AndroidVrClient, ios::IosClient,
    music_android::MusicAndroidClient, tv::TvClient, tv_cast::TvCastClient,
    tv_embedded::TvEmbeddedClient, web::WebClient, web_embedded::WebEmbeddedClient,
    web_parent_tools::WebParentToolsClient, web_remix::WebRemixClient,
};
use oauth::YouTubeOAuth;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct YouTubeStreamInfo {
    /// Resolved direct or HLS stream URL.
    pub url: String,
    /// Protocol: "http" or "hls".
    pub protocol: String,
    /// Simplified format string (e.g. "webm/opus" or "mp4/aac").
    pub format: String,
    /// HLS manifest URL (if applicable).
    pub hls_url: Option<String>,
    /// All available formats from `streamingData`.
    pub formats: Vec<YouTubeFormatInfo>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct YouTubeFormatInfo {
    pub itag: i64,
    pub mime_type: String,
    pub bitrate: i64,
    pub quality_label: Option<String>,
    pub audio_quality: Option<String>,
}

pub struct YouTubeSource {
    search_prefixes: Vec<String>,
    rec_prefixes: Vec<String>,
    url_regex: Regex,
    // Store clients separated by function
    search_clients: Vec<Arc<dyn YouTubeClient>>,
    playback_clients: Vec<Arc<dyn YouTubeClient>>,
    resolve_clients: Vec<Arc<dyn YouTubeClient>>,
    oauth: Arc<YouTubeOAuth>,
    cipher_manager: Arc<YouTubeCipherManager>,
    visitor_data: SharedRw<Option<String>>,
    #[allow(dead_code)]
    http: Arc<reqwest::Client>,
}

impl YouTubeSource {
    pub fn new(config: Option<YouTubeConfig>, http: Arc<reqwest::Client>) -> Self {
        let config = config.unwrap_or_default();
        let oauth = Arc::new(YouTubeOAuth::new(config.clients.refresh_tokens.clone()));
        let cipher_manager = Arc::new(YouTubeCipherManager::new(config.cipher.clone()));

        // Call initialization in background if no tokens provided and enabled in config
        if config.clients.get_oauth_token && config.clients.refresh_tokens.is_empty() {
            let oauth_clone = oauth.clone();
            tokio::spawn(async move {
                oauth_clone.initialize_access_token().await;
            });
        }

        // Warm the cipher cache on startup (Issue #21)
        let cm_clone = cipher_manager.clone();
        tokio::spawn(async move {
            debug!("YouTubeSource: Warming cipher cache...");
            if let Err(e) = cm_clone.get_cached_player_script().await {
                warn!("YouTubeSource: Failed to warm cipher cache: {}", e);
            } else {
                debug!("YouTubeSource: Cipher cache warmed.");
            }
        });

        let visitor_data = Arc::new(RwLock::new(None));

        let vd_clone = visitor_data.clone();
        let http_clone = http.clone();
        tokio::spawn(async move {
            loop {
                if let Some(vd) = Self::refresh_visitor_data(&http_clone).await {
                    let mut lock = vd_clone.write().await;
                    *lock = Some(vd);
                    tracing::debug!("YouTube visitorData refreshed.");
                } else {
                    tracing::warn!("Failed to refresh YouTube visitorData.");
                }
                tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
            }
        });

        let cipher_url = config.cipher.url.clone();
        let cipher_token = config.cipher.token.clone();
        let create_client = |name: &str| -> Option<Arc<dyn YouTubeClient>> {
            match Self::canonicalize_youtube_client(name) {
                Some("WEB") => Some(Arc::new(WebClient::with_cipher_url(
                    http.clone(),
                    cipher_url.clone(),
                    cipher_token.clone(),
                ))),
                Some("WEB_REMIX") => Some(Arc::new(WebRemixClient::new(http.clone()))),
                Some("ANDROID") => Some(Arc::new(AndroidClient::new(http.clone()))),
                Some("IOS") => Some(Arc::new(IosClient::new(http.clone()))),
                Some("TVHTML5") => Some(Arc::new(TvClient::new(http.clone()))),
                Some("TVHTML5_CAST") => Some(Arc::new(TvCastClient::new(http.clone()))),
                Some("TVHTML5_SIMPLY_EMBEDDED_PLAYER") => {
                    Some(Arc::new(TvEmbeddedClient::new(http.clone())))
                }
                Some("ANDROID_MUSIC") => {
                    Some(Arc::new(MusicAndroidClient::new(http.clone())))
                }
                Some("ANDROID_VR") => Some(Arc::new(AndroidVrClient::new(http.clone()))),
                Some("WEB_EMBEDDED_PLAYER") => {
                    Some(Arc::new(WebEmbeddedClient::new(http.clone())))
                }
                Some("WEB_PARENT_TOOLS") => {
                    Some(Arc::new(WebParentToolsClient::new(http.clone())))
                }
                _ => {
                    tracing::warn!("Unknown YouTube client: {}", name);
                    None
                }
            }
        };

        let mut search_clients = Vec::new();
        for name in &config.clients.search {
            if let Some(client) = create_client(name) {
                search_clients.push(client);
            }
        }
        if search_clients.is_empty() {
            tracing::warn!("No valid YouTube search clients configured! Fallback to Web.");
            search_clients.push(Arc::new(WebClient::new(http.clone())));
        }

        let mut playback_clients = Vec::new();
        for name in &config.clients.playback {
            if let Some(client) = create_client(name) {
                playback_clients.push(client);
            }
        }

        if playback_clients.is_empty() {
            tracing::warn!("No valid YouTube playback clients configured! Fallback to Web.");
            playback_clients.push(Arc::new(WebClient::new(http.clone())));
        }

        let mut resolve_clients = Vec::new();
        for name in &config.clients.resolve {
            if let Some(client) = create_client(name) {
                resolve_clients.push(client);
            }
        }
        if resolve_clients.is_empty() {
            tracing::warn!("No valid YouTube resolve clients configured! Fallback to Web.");
            resolve_clients.push(Arc::new(WebClient::new(http.clone())));
        }

        tracing::info!(
            "YouTube source initialized with {} search, {} playback, and {} resolve clients.",
            search_clients.len(),
            playback_clients.len(),
            resolve_clients.len()
        );

        Self {
            search_prefixes: vec!["ytsearch:".to_string(), "ytmsearch:".to_string()],
            rec_prefixes: vec!["ytrec:".to_string()],
            url_regex: Regex::new(r"(?:youtube\.com|youtu\.be)").unwrap(),
            search_clients,
            playback_clients,
            resolve_clients,
            oauth,
            cipher_manager,
            visitor_data,
            http,
        }
    }

    async fn refresh_visitor_data(http: &reqwest::Client) -> Option<String> {
        let body = json!({
            "context": {
                "client": {
                    "clientName": "WEB",
                    "clientVersion": "2.20260114.01.00",
                    "hl": "en",
                    "gl": "US"
                }
            }
        });

        match http
            .post("https://www.youtube.com/youtubei/v1/guide")
            .json(&body)
            .send()
            .await
        {
            Ok(res) => {
                if let Ok(json) = res.json::<Value>().await {
                    if let Some(vd) = json
                        .get("responseContext")
                        .and_then(|rc| rc.get("visitorData"))
                        .and_then(|vd| vd.as_str())
                    {
                        // Always URL-decode to ensure clean base64 is stored (no %3D%3D etc.)
                        let decoded = urlencoding::decode(vd)
                            .map(|s| s.into_owned())
                            .unwrap_or_else(|_| vd.to_string());
                        return Some(decoded);
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Failed to fetch visitor data: {}", e);
            }
        }
        None
    }

    fn extract_playlist_id(&self, identifier: &str) -> Option<String> {
        if identifier.contains("list=") {
            return Some(
                identifier
                    .split("list=")
                    .nth(1)
                    .unwrap_or(identifier)
                    .split('&')
                    .next()
                    .unwrap_or(identifier)
                    .to_string(),
            );
        }
        None
    }

    fn extract_id(&self, identifier: &str) -> String {
        if identifier.contains("v=") {
            identifier
                .split("v=")
                .nth(1)
                .unwrap_or(identifier)
                .split('&')
                .next()
                .unwrap_or(identifier)
                .to_string()
        } else if identifier.contains("youtu.be/") {
            identifier
                .split("youtu.be/")
                .nth(1)
                .unwrap_or(identifier)
                .split('?')
                .next()
                .unwrap_or(identifier)
                .to_string()
        } else if identifier.contains("/live/") {
            identifier
                .split("/live/")
                .nth(1)
                .unwrap_or(identifier)
                .split('?')
                .next()
                .unwrap_or(identifier)
                .to_string()
        } else if identifier.contains("/shorts/") {
            identifier
                .split("/shorts/")
                .nth(1)
                .unwrap_or(identifier)
                .split('?')
                .next()
                .unwrap_or(identifier)
                .to_string()
        } else {
            identifier.to_string()
        }
    }

    fn prioritize_clients<'a>(
        &'a self,
        clients: &'a [Arc<dyn YouTubeClient>],
        prefer_music: bool,
    ) -> Vec<&'a Arc<dyn YouTubeClient>> {
        let mut ordered = Vec::with_capacity(clients.len());
        if prefer_music {
            ordered.extend(
                clients
                    .iter()
                    .filter(|c| c.name().contains("Music") || c.name().contains("Remix")),
            );
            ordered.extend(
                clients
                    .iter()
                    .filter(|c| !c.name().contains("Music") && !c.name().contains("Remix")),
            );
        } else {
            ordered.extend(
                clients
                    .iter()
                    .filter(|c| !c.name().contains("Music") && !c.name().contains("Remix")),
            );
            ordered.extend(
                clients
                    .iter()
                    .filter(|c| c.name().contains("Music") || c.name().contains("Remix")),
            );
        }
        ordered
    }

    pub fn cipher_manager(&self) -> Arc<YouTubeCipherManager> {
        self.cipher_manager.clone()
    }

    /// Resolve a direct stream URL for a YouTube video ID, for use by the /v4/trackstream endpoint.
    /// WEB client is explicitly excluded (uses SABR binary protocol).
    pub async fn get_stream_info(
        &self,
        video_id: &str,
        itag: Option<i64>,
        with_client: Option<&str>,
    ) -> Result<YouTubeStreamInfo, String> {
        let visitor_data = self.visitor_data.read().await.clone();
        let context = if let Some(ref vd) = visitor_data {
            json!({ "visitorData": vd })
        } else {
            json!({})
        };

        // Filter playback clients: exclude WEB (SABR), optionally filter to a specific named client.
        let clients: Vec<&Arc<dyn YouTubeClient>> = self
            .playback_clients
            .iter()
            .filter(|c| {
                let n = c.name();
                // Always exclude the WEB client (SABR protocol)
                if n.eq_ignore_ascii_case("Web") {
                    return false;
                }
                // If with_client specified, only allow that client name
                if let Some(wc) = with_client {
                    return n.eq_ignore_ascii_case(wc)
                        || c.client_name().eq_ignore_ascii_case(wc);
                }
                true
            })
            .collect();

        if clients.is_empty() {
            return Err(format!(
                "No eligible YouTube clients available{}",
                with_client.map(|c| format!(" for client '{}'" , c)).unwrap_or_default()
            ));
        }

        let player_page_url = format!("https://www.youtube.com/watch?v={}", video_id);

        for client in clients {
            let streaming_data = match client
                .get_streaming_data(
                    video_id,
                    &context,
                    self.cipher_manager.clone(),
                    self.oauth.clone(),
                )
                .await
            {
                Ok(Some(sd)) => sd,
                Ok(None) => continue,
                Err(e) => {
                    tracing::warn!("YouTube trackstream: client '{}' error: {}", client.name(), e);
                    continue;
                }
            };

            // HLS manifest (live streams)
            if let Some(hls_url) = streaming_data
                .get("hlsManifestUrl")
                .and_then(|v| v.as_str())
            {
                let formats = Self::collect_formats(&streaming_data);
                return Ok(YouTubeStreamInfo {
                    url: hls_url.to_string(),
                    protocol: "hls".to_string(),
                    format: "m3u8".to_string(),
                    hls_url: Some(hls_url.to_string()),
                    formats,
                });
            }

            let adaptive = streaming_data.get("adaptiveFormats").and_then(|v| v.as_array());
            let formats_arr = streaming_data.get("formats").and_then(|v| v.as_array());

            // If a specific itag was requested, find that format
            if let Some(target_itag) = itag {
                let all_formats: Vec<&Value> = adaptive
                    .iter()
                    .flat_map(|a| a.iter())
                    .chain(formats_arr.iter().flat_map(|a| a.iter()))
                    .collect();

                let found = all_formats
                    .iter()
                    .find(|f| f.get("itag").and_then(|v| v.as_i64()) == Some(target_itag));

                if let Some(fmt) = found {
                    match clients::common::resolve_format_url(
                        fmt,
                        &player_page_url,
                        &self.cipher_manager,
                    )
                    .await
                    {
                        Ok(Some(url)) => {
                            let all_infos = Self::collect_formats(&streaming_data);
                            let format = fmt
                                .get("mimeType")
                                .and_then(|v| v.as_str())
                                .map(utils::simplify_mime)
                                .unwrap_or_else(|| "mp4/aac".to_string());
                            return Ok(YouTubeStreamInfo {
                                url,
                                protocol: "http".to_string(),
                                format,
                                hls_url: None,
                                formats: all_infos,
                            });
                        }
                        Ok(None) => continue,
                        Err(e) => {
                            tracing::warn!(
                                "YouTube trackstream: itag {} cipher error: {}",
                                target_itag,
                                e
                            );
                            continue;
                        }
                    }
                }
                // itag not found in this client's response, try next client
                continue;
            }

            // No itag specified: use best audio format
            if let Some(best) = clients::common::select_best_audio_format(adaptive, formats_arr) {
                match clients::common::resolve_format_url(
                    best,
                    &player_page_url,
                    &self.cipher_manager,
                )
                .await
                {
                    Ok(Some(url)) => {
                        let all_infos = Self::collect_formats(&streaming_data);
                        let format = best
                            .get("mimeType")
                            .and_then(|v| v.as_str())
                            .map(utils::simplify_mime)
                            .unwrap_or_else(|| "mp4/aac".to_string());
                        return Ok(YouTubeStreamInfo {
                            url,
                            protocol: "http".to_string(),
                            format,
                            hls_url: None,
                            formats: all_infos,
                        });
                    }
                    Ok(None) => continue,
                    Err(e) => {
                        tracing::warn!("YouTube trackstream: best format error: {}", e);
                        continue;
                    }
                }
            }
        }

        if let Some(target_itag) = itag {
            Err(format!("itag {} not found or no client could resolve it", target_itag))
        } else {
            Err("No client could resolve a stream URL for this video".to_string())
        }
    }

    fn collect_formats(streaming_data: &Value) -> Vec<YouTubeFormatInfo> {
        let mut formats = Vec::new();
        for arr_key in &["adaptiveFormats", "formats"] {
            if let Some(arr) = streaming_data.get(arr_key).and_then(|v| v.as_array()) {
                for f in arr {
                    let itag = match f.get("itag").and_then(|v| v.as_i64()) {
                        Some(i) => i,
                        None => continue,
                    };
                    formats.push(YouTubeFormatInfo {
                        itag,
                        mime_type: f
                            .get("mimeType")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        bitrate: f
                            .get("bitrate")
                            .and_then(|v| v.as_i64())
                            .unwrap_or(0),
                        quality_label: f
                            .get("qualityLabel")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string()),
                        audio_quality: f
                            .get("audioQuality")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string()),
                    });
                }
            }
        }
        formats
    }
}

#[async_trait]
impl SourcePlugin for YouTubeSource {
    fn name(&self) -> &str {
        "youtube"
    }

    fn can_handle(&self, identifier: &str) -> bool {
        self.search_prefixes
            .iter()
            .any(|p| identifier.starts_with(p))
            || self.rec_prefixes.iter().any(|p| identifier.starts_with(p))
            || self.url_regex.is_match(identifier)
    }

    fn search_prefixes(&self) -> Vec<&str> {
        self.search_prefixes.iter().map(|s| s.as_str()).collect()
    }

    async fn load(
        &self,
        identifier: &str,
        _routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> LoadResult {
        let visitor_data = self.visitor_data.read().await.clone();
        let context = if let Some(vd) = visitor_data {
            json!({ "visitorData": vd })
        } else {
            json!({})
        };

        if let Some(prefix) = self
            .search_prefixes
            .iter()
            .find(|p| identifier.starts_with(*p))
        {
            return self.handle_search(identifier, prefix, &context).await;
        }

        if let Some(prefix) = self
            .rec_prefixes
            .iter()
            .find(|p| identifier.starts_with(*p))
        {
            return self
                .handle_recommendations(identifier, prefix, &context)
                .await;
        }

        if self.url_regex.is_match(identifier) {
            return self.handle_url(identifier, &context).await;
        }

        LoadResult::Empty {}
    }

    async fn get_track(
        &self,
        identifier: &str,
        routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<BoxedTrack> {
        let visitor_data = self.visitor_data.read().await.clone();
        let id = self.extract_id(identifier);
        let is_music_url = identifier.contains("music.youtube.com");

        let clients_to_try = self.prioritize_clients(&self.playback_clients, is_music_url);
        let clients = clients_to_try.into_iter().cloned().collect();

        Some(Box::new(track::YoutubeTrack {
            identifier: id,
            clients,
            oauth: self.oauth.clone(),
            cipher_manager: self.cipher_manager.clone(),
            visitor_data,
            local_addr: routeplanner.and_then(|rp| rp.get_address()),
            proxy: None,
        }))
    }

    async fn get_stream_url(&self, identifier: &str, itag: Option<i64>) -> Option<StreamInfo> {
        let video_id = self.extract_id(identifier);
        let info = self.get_stream_info(&video_id, itag, None).await.ok()?;

        let protocol = if info.url.contains(".m3u8") || info.url.contains("/hls/") {
            "hls"
        } else {
            "http"
        };

        let mime_type = if protocol == "hls" {
            "application/x-mpegURL".to_string()
        } else {
            let path = info.url.split('?').next().unwrap_or(&info.url);
            match std::path::Path::new(path)
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_ascii_lowercase())
                .as_deref()
            {
                Some("m4a") => "audio/mp4",
                Some("mp3") => "audio/mpeg",
                Some("opus") => "audio/opus",
                Some("ogg") | Some("oga") => "audio/ogg",
                Some("webm") => "audio/webm",
                _ => "audio/mp4",
            }
            .to_string()
        };

        Some(StreamInfo {
            url: info.url,
            mime_type,
            protocol: protocol.to_string(),
        })
    }
}

impl YouTubeSource {
    async fn handle_search(&self, identifier: &str, prefix: &str, context: &Value) -> LoadResult {
        let (query, prefer_music) = if prefix == "ytmsearch:" {
            (&identifier[prefix.len()..], true)
        } else {
            (&identifier[prefix.len()..], false)
        };

        let clients = self.prioritize_clients(&self.search_clients, prefer_music);
        for client in clients {
            tracing::debug!("Searching '{}' with {}", query, client.name());
            match client.search(query, context, self.oauth.clone()).await {
                Ok(tracks) if !tracks.is_empty() => return LoadResult::Search(tracks),
                Ok(_) => continue,
                Err(e) => tracing::error!("Search error with {}: {}", client.name(), e),
            }
        }
        LoadResult::Empty {}
    }

    async fn handle_recommendations(
        &self,
        identifier: &str,
        prefix: &str,
        context: &Value,
    ) -> LoadResult {
        let seed_id = &identifier[prefix.len()..];
        let playlist_id = format!("RD{}", seed_id);

        let clients = self.prioritize_clients(&self.resolve_clients, true);
        for client in clients {
            match client
                .get_playlist(&playlist_id, context, self.oauth.clone())
                .await
            {
                Ok(Some((tracks, title))) => {
                    let filtered: Vec<Track> = tracks
                        .into_iter()
                        .filter(|t| t.info.identifier != seed_id)
                        .collect();
                    return LoadResult::Playlist(PlaylistData {
                        info: PlaylistInfo {
                            name: format!("Recommendations: {}", title),
                            selected_track: -1,
                        },
                        plugin_info: json!({
                          "type": "recommendations",
                          "totalTracks": filtered.len()
                        }),
                        tracks: filtered,
                    });
                }
                _ => continue,
            }
        }
        LoadResult::Empty {}
    }

    async fn handle_url(&self, identifier: &str, context: &Value) -> LoadResult {
        let is_music_url = identifier.contains("music.youtube.com");

        // Playlist handling
        if let Some(playlist_id) = self.extract_playlist_id(identifier) {
            let mut clients = Vec::new();

            // Try Android first as it's most reliable for playlists
            if let Some(android) = self
                .resolve_clients
                .iter()
                .chain(self.search_clients.iter())
                .find(|c| c.name() == "Android")
            {
                clients.push(android);
            }

            for c in self.prioritize_clients(&self.resolve_clients, is_music_url) {
                if !clients.iter().any(|&x| x.name() == c.name()) {
                    clients.push(c);
                }
            }

            for client in clients {
                match client
                    .get_playlist(&playlist_id, context, self.oauth.clone())
                    .await
                {
                    Ok(Some((tracks, title))) => {
                        return LoadResult::Playlist(PlaylistData {
                            info: PlaylistInfo {
                                name: title,
                                selected_track: -1,
                            },
                            plugin_info: json!({
                              "type": "playlist",
                              "url": format!("https://www.youtube.com/playlist?list={}", playlist_id),
                              "artworkUrl": tracks.first().and_then(|t| t.info.artwork_url.clone()),
                              "totalTracks": tracks.len()
                            }),
                            tracks,
                        });
                    }
                    _ => continue,
                }
            }
        }

        // Track info handling
        let id = self.extract_id(identifier);

        let resolve_clients = self.prioritize_clients(&self.resolve_clients, is_music_url);
        for client in &resolve_clients {
            match client
                .get_track_info(&id, context, self.oauth.clone())
                .await
            {
                Ok(Some(track)) => return LoadResult::Track(track),
                _ => continue,
            }
        }

        // Partial Fallback to Playback Clients for info
        let playback_clients = self.prioritize_clients(&self.playback_clients, is_music_url);
        for client in playback_clients {
            if resolve_clients.iter().any(|&rc| rc.name() == client.name()) {
                continue;
            }
            if let Ok(Some(track)) = client
                .get_track_info(&id, context, self.oauth.clone())
                .await
            {
                return LoadResult::Track(track);
            }
        }

        LoadResult::Empty {}
    }

    pub fn canonicalize_youtube_client(name: &str) -> Option<&'static str> {
        match name.to_uppercase().as_str() {
            "WEB" => Some("WEB"),
            "MWEB" | "REMIX" | "MUSIC_WEB" | "WEB_REMIX" => Some("WEB_REMIX"),
            "ANDROID" => Some("ANDROID"),
            "IOS" => Some("IOS"),
            "TV" | "TVHTML5" => Some("TVHTML5"),
            "TV_CAST" | "TVHTML5_CAST" => Some("TVHTML5_CAST"),
            "TV_EMBEDDED" | "TVHTML5_EMBEDDED" | "TVHTML5_SIMPLY" | "TVHTML5_SIMPLY_EMBEDDED_PLAYER" => {
                Some("TVHTML5_SIMPLY_EMBEDDED_PLAYER")
            }
            "MUSIC" | "MUSIC_ANDROID" | "ANDROID_MUSIC" => Some("ANDROID_MUSIC"),
            "ANDROID_VR" | "ANDROIDVR" => Some("ANDROID_VR"),
            "WEB_EMBEDDED" | "WEBEMBEDDED" | "WEB_EMBEDDED_PLAYER" => Some("WEB_EMBEDDED_PLAYER"),
            "WEB_PARENT_TOOLS" | "WEBPARENTTOOLS" => Some("WEB_PARENT_TOOLS"),
            _ => None,
        }
    }
}
