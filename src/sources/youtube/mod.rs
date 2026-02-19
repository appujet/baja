use crate::api::tracks::{LoadResult, PlaylistData, PlaylistInfo, Track};
use crate::configs::sources::YouTubeConfig;
use crate::sources::SourcePlugin;
use async_trait::async_trait;
use regex::Regex;
use std::sync::Arc;

pub mod cipher;
pub mod clients;
pub mod extractor;
pub mod oauth;
pub mod sabr;

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
    search_clients: Vec<Box<dyn YouTubeClient>>,
    playback_clients: Vec<Box<dyn YouTubeClient>>,
    oauth: Arc<YouTubeOAuth>,
    cipher_manager: Arc<YouTubeCipherManager>,
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

        let create_client = |name: &str| -> Option<Box<dyn YouTubeClient>> {
            match name.to_uppercase().as_str() {
                "WEB" => Some(Box::new(WebClient::new())),
                "ANDROID" => Some(Box::new(AndroidClient::new())),
                "IOS" => Some(Box::new(IosClient::new())),
                "TV" => Some(Box::new(TvClient::new())),
                "MUSIC" | "MUSIC_ANDROID" => Some(Box::new(MusicAndroidClient::new())),
                "WEB_REMIX" | "MUSIC_WEB" => Some(Box::new(WebRemixClient::new())),
                "ANDROID_VR" | "ANDROIDVR" => Some(Box::new(AndroidVrClient::new())),
                "WEB_EMBEDDED" | "WEBEMBEDDED" => Some(Box::new(WebEmbeddedClient::new())),
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
            search_clients.push(Box::new(WebClient::new()));
        }

        let mut playback_clients = Vec::new();
        for name in &config.clients.playback {
            if let Some(client) = create_client(name) {
                playback_clients.push(client);
            }
        }
        if playback_clients.is_empty() {
            tracing::warn!("No valid YouTube playback clients configured! Fallback to Web.");
            playback_clients.push(Box::new(WebClient::new()));
        }

        Self {
            search_prefix: "ytsearch:".to_string(),
            url_regex: Regex::new(r"(?:youtube\.com|youtu\.be)").unwrap(),
            search_clients,
            playback_clients,
            oauth,
            cipher_manager,
        }
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
        clients: &'a [Box<dyn YouTubeClient>],
        prefer_music: bool,
    ) -> Vec<&'a Box<dyn YouTubeClient>> {
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
        let context = serde_json::json!({});

        if identifier.starts_with(&self.search_prefix) || identifier.starts_with("ytmsearch:") {
            let (query, search_type) = if identifier.starts_with("ytmsearch:") {
                (&identifier["ytmsearch:".len()..], "music")
            } else {
                (&identifier["ytsearch:".len()..], "video")
            };

            tracing::debug!("Searching for: {} (type: {})", query, search_type);

            let clients_to_try =
                self.prioritize_clients(&self.search_clients, search_type == "music");

            for client in clients_to_try {
                tracing::debug!("Trying search client: {}", client.name());
                match client.search(query, &context, self.oauth.clone()).await {
                    Ok(tracks) => {
                        if !tracks.is_empty() {
                            tracing::debug!("Found {} tracks with {}", tracks.len(), client.name());
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
                match client.get_playlist(&playlist_id, self.oauth.clone()).await {
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
                    tracing::debug!("Trying playlist client: {}", client.name());
                    match client.get_playlist(&playlist_id, self.oauth.clone()).await {
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

            // Fallback to track loading if playlist failed or was skipped
            let clients_to_try = self.prioritize_clients(&self.playback_clients, is_music_url);

            for client in clients_to_try {
                tracing::debug!("Trying track info client: {}", client.name());
                match client.get_track_info(&id, self.oauth.clone()).await {
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

    async fn get_playback_url(
        &self,
        identifier: &str,
        _routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<String> {
        let context = serde_json::json!({});
        let id = self.extract_id(identifier);
        let is_music_url = identifier.contains("music.youtube.com");

        let clients_to_try = self.prioritize_clients(&self.playback_clients, is_music_url);

        for client in clients_to_try {
            tracing::debug!("Trying playback URL client: {}", client.name());
            match client
                .get_track_url(
                    &id,
                    &context,
                    self.cipher_manager.clone(),
                    self.oauth.clone(),
                )
                .await
            {
                Ok(Some(url)) => {
                    tracing::debug!("Resolved playback URL with {}", client.name());
                    return Some(url);
                }
                Ok(None) => continue,
                Err(e) => {
                    tracing::error!("Playback URL error with {}: {}", client.name(), e);
                }
            }
        }
        None
    }
}
