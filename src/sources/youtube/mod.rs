use crate::api::tracks::{LoadResult, PlaylistData, PlaylistInfo, Track};
use crate::configs::sources::YouTubeConfig;
use crate::sources::SourcePlugin;
use async_trait::async_trait;
use regex::Regex;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::RwLock;

pub mod cipher;
pub mod clients;
pub mod extractor;
pub mod hls;
pub mod oauth;
pub mod sabr;
pub mod track;

use cipher::YouTubeCipherManager;
use clients::YouTubeClient;
use clients::android::AndroidClient;
use clients::android_vr::AndroidVrClient;
use clients::ios::IosClient;
use clients::music_android::MusicAndroidClient;
use clients::tv::TvClient;
use clients::web::WebClient;
use clients::web_embedded::WebEmbeddedClient;
use clients::web_remix::WebRemixClient;
use oauth::YouTubeOAuth;

pub struct YouTubeSource {
    search_prefix: String,
    url_regex: Regex,
    // Store clients separated by function
    search_clients: Vec<Arc<dyn YouTubeClient>>,
    playback_clients: Vec<Arc<dyn YouTubeClient>>,
    oauth: Arc<YouTubeOAuth>,
    cipher_manager: Arc<YouTubeCipherManager>,
    visitor_data: Arc<RwLock<Option<String>>>,
    #[allow(dead_code)]
    http: reqwest::Client,
}

impl YouTubeSource {
    pub fn new(config: Option<YouTubeConfig>) -> Self {
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

        let visitor_data = Arc::new(RwLock::new(None));
        let http = reqwest::Client::builder()
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/121.0.0.0 Safari/537.36")
            .build()
            .unwrap_or_default();

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

        let create_client = |name: &str| -> Option<Arc<dyn YouTubeClient>> {
            match name.to_uppercase().as_str() {
                "WEB" => Some(Arc::new(WebClient::new())),
                "ANDROID" => Some(Arc::new(AndroidClient::new())),
                "IOS" => Some(Arc::new(IosClient::new())),
                "TV" => Some(Arc::new(TvClient::new())),
                "MUSIC" | "MUSIC_ANDROID" => Some(Arc::new(MusicAndroidClient::new())),
                "WEB_REMIX" | "MUSIC_WEB" => Some(Arc::new(WebRemixClient::new())),
                "ANDROID_VR" | "ANDROIDVR" => Some(Arc::new(AndroidVrClient::new())),
                "WEB_EMBEDDED" | "WEBEMBEDDED" => Some(Arc::new(WebEmbeddedClient::new())),
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
            search_clients.push(Arc::new(WebClient::new()));
        }

        let mut playback_clients = Vec::new();
        for name in &config.clients.playback {
            if let Some(client) = create_client(name) {
                playback_clients.push(client);
            }
        }
        if playback_clients.is_empty() {
            tracing::warn!("No valid YouTube playback clients configured! Fallback to Web.");
            playback_clients.push(Arc::new(WebClient::new()));
        }

        Self {
            search_prefix: "ytsearch:".to_string(),
            url_regex: Regex::new(r"(?:youtube\.com|youtu\.be)").unwrap(),
            search_clients,
            playback_clients,
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
            ordered.extend(clients.iter().filter(|c| c.name().starts_with("Music")));
            ordered.extend(clients.iter().filter(|c| !c.name().starts_with("Music")));
        } else {
            ordered.extend(clients.iter().filter(|c| !c.name().starts_with("Music")));
            ordered.extend(clients.iter().filter(|c| c.name().starts_with("Music")));
        }
        ordered
    }

    pub fn cipher_manager(&self) -> Arc<YouTubeCipherManager> {
        self.cipher_manager.clone()
    }
}

#[async_trait]
impl SourcePlugin for YouTubeSource {
    fn name(&self) -> &str {
        "youtube"
    }

    fn can_handle(&self, identifier: &str) -> bool {
        identifier.starts_with(&self.search_prefix)
            || identifier.starts_with("ytmsearch:")
            || identifier.starts_with("ytrec:")
            || self.url_regex.is_match(identifier)
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

        if identifier.starts_with(&self.search_prefix) || identifier.starts_with("ytmsearch:") {
            let (query, search_type) = if identifier.starts_with("ytmsearch:") {
                (&identifier["ytmsearch:".len()..], "music")
            } else {
                (&identifier["ytsearch:".len()..], "video")
            };

            let clients_to_try =
                self.prioritize_clients(&self.search_clients, search_type == "music");

            for client in clients_to_try {
                tracing::debug!("Searching for: {} using {}", query, client.name());
                match client.search(query, &context, self.oauth.clone()).await {
                    Ok(tracks) => {
                        if !tracks.is_empty() {
                            return LoadResult::Search(tracks);
                        }
                    }
                    Err(e) => {
                        tracing::error!("Search error with {}: {}", client.name(), e);
                    }
                }
            }
            return LoadResult::Empty {};
        }

        if identifier.starts_with("ytrec:") {
            let seed_id = &identifier["ytrec:".len()..];
            let playlist_id = format!("RD{}", seed_id);

            // Recommendations are almost always music related
            let clients_to_try = self.prioritize_clients(&self.search_clients, true);

            for client in clients_to_try {
                match client
                    .get_playlist(&playlist_id, &context, self.oauth.clone())
                    .await
                {
                    Ok(Some((tracks, title))) => {
                        let filtered_tracks: Vec<Track> = tracks
                            .into_iter()
                            .filter(|t| t.info.identifier != seed_id)
                            .collect();
                        return LoadResult::Playlist(PlaylistData {
                            info: PlaylistInfo {
                                name: format!("Recommendations: {}", title),
                                selected_track: -1,
                            },
                            plugin_info: serde_json::json!({ "type": "recommendations" }),
                            tracks: filtered_tracks,
                        });
                    }
                    Ok(None) => continue,
                    Err(e) => {
                        tracing::error!("Recommendations error with {}: {}", client.name(), e);
                    }
                }
            }
            return LoadResult::Empty {};
        }

        if self.url_regex.is_match(identifier) {
            let id = self.extract_id(identifier);
            let is_music_url = identifier.contains("music.youtube.com");

            // First check for playlist
            if let Some(playlist_id) = self.extract_playlist_id(identifier) {
                let clients_to_try = self.prioritize_clients(&self.search_clients, is_music_url);

                for client in clients_to_try {
                    tracing::debug!("Loading playlist '{}' using {}", playlist_id, client.name());
                    match client
                        .get_playlist(&playlist_id, &context, self.oauth.clone())
                        .await
                    {
                        Ok(Some((tracks, title))) => {
                            tracing::debug!(
                                "Successfully loaded playlist '{}' with {}",
                                title,
                                client.name()
                            );
                            return LoadResult::Playlist(PlaylistData {
                                info: PlaylistInfo {
                                    name: title,
                                    selected_track: -1,
                                },
                                plugin_info: serde_json::json!({}),
                                tracks,
                            });
                        }
                        Ok(None) => {
                            tracing::debug!(
                                "Client {} returned no playlist for {}",
                                client.name(),
                                playlist_id
                            );
                            continue;
                        }
                        Err(e) => {
                            tracing::error!("Playlist error with {}: {}", client.name(), e);
                        }
                    }
                }
            }

            let clients_to_try = self.prioritize_clients(&self.playback_clients, is_music_url);

            for client in clients_to_try {
                tracing::debug!("Loading track '{}' using {}", id, client.name());
                match client
                    .get_track_info(&id, &context, self.oauth.clone())
                    .await
                {
                    Ok(Some(track)) => {
                        tracing::debug!("Successfully loaded track {} with {}", id, client.name());
                        return LoadResult::Track(track);
                    }
                    Ok(None) => continue,
                    Err(e) => {
                        tracing::error!("Track info error with {}: {}", client.name(), e);
                    }
                }
            }
        }

        LoadResult::Empty {}
    }

    async fn get_track(
        &self,
        identifier: &str,
        routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<Box<dyn crate::sources::plugin::PlayableTrack>> {
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
            proxy: None, // YoutubeSource doesn't seem to use proxy the same way yet
        }))
    }
}
