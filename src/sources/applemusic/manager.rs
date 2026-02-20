use std::sync::Arc;

use async_trait::async_trait;
use regex::Regex;
use reqwest::header::{HeaderMap, HeaderValue, USER_AGENT};
use serde_json::Value;
use tracing::{error, warn};

use super::token::AppleMusicTokenTracker;
use crate::{
    api::tracks::{LoadResult, PlaylistData, PlaylistInfo, Track, TrackInfo},
    sources::SourcePlugin,
};

const API_BASE: &str = "https://api.music.apple.com/v1";
pub struct AppleMusicSource {
    client: reqwest::Client,
    token_tracker: Arc<AppleMusicTokenTracker>,
    country_code: String,

    #[allow(dead_code)]
    playlist_load_limit: usize,
    #[allow(dead_code)]
    album_load_limit: usize,
    #[allow(dead_code)]
    playlist_page_load_concurrency: usize,
    #[allow(dead_code)]
    album_page_load_concurrency: usize,

    search_prefix: String,
    url_regex: Regex,
}

impl AppleMusicSource {
    pub fn new(config: Option<crate::configs::AppleMusicConfig>) -> Self {
        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, HeaderValue::from_static("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36"));

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .build()
            .unwrap();

        let (country, p_limit, a_limit, p_conc, a_conc) = if let Some(c) = config {
            (
                c.country_code,
                c.playlist_load_limit,
                c.album_load_limit,
                c.playlist_page_load_concurrency,
                c.album_page_load_concurrency,
            )
        } else {
            ("us".to_string(), 0, 0, 5, 5)
        };

        Self {
            token_tracker: Arc::new(crate::sources::applemusic::token::AppleMusicTokenTracker::new(client.clone())),
            client,
            country_code: country,
            playlist_load_limit: p_limit,
            album_load_limit: a_limit,
            playlist_page_load_concurrency: p_conc,
            album_page_load_concurrency: a_conc,
            search_prefix: "amsearch:".to_string(),
            url_regex: Regex::new(r"https?://(?:www\.)?music\.apple\.com/(?:[a-zA-Z]{2}/)?(album|playlist|artist|song)/[^/]+/([a-zA-Z0-9\-.]+)(?:\?i=(\d+))?").unwrap(),
        }
    }

    async fn api_request(&self, path: &str) -> Option<Value> {
        let token = self.token_tracker.get_token().await?;
        let origin = self.token_tracker.get_origin().await;

        let url = if path.starts_with("http") {
            path.to_string()
        } else {
            format!("{}{}", API_BASE, path)
        };

        let mut req = self.client.get(&url).bearer_auth(token);

        if let Some(o) = origin {
            req = req.header("Origin", format!("https://{}", o));
        }

        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                error!("Apple Music API request failed: {}", e);
                return None;
            }
        };

        if !resp.status().is_success() {
            warn!("Apple Music API returned {}", resp.status());
            return None;
        }

        resp.json().await.ok()
    }

    fn build_track(&self, item: &Value, artwork_override: Option<String>) -> Option<TrackInfo> {
        let attributes = item.get("attributes")?;

        let id = item.get("id")?.as_str()?.to_string();
        let title = attributes
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown Title")
            .to_string();
        let author = attributes
            .get("artistName")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown Artist")
            .to_string();
        let length = attributes
            .get("durationInMillis")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        let isrc = attributes
            .get("isrc")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let artwork_url = artwork_override.or_else(|| {
            attributes
                .get("artwork")
                .and_then(|a| a.get("url"))
                .and_then(|v| v.as_str())
                .map(|s| s.replace("{w}", "1000").replace("{h}", "1000"))
        });

        let url = attributes
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        Some(TrackInfo {
            title,
            author,
            length,
            identifier: id,
            is_stream: false,
            uri: Some(url),
            artwork_url,
            isrc,
            source_name: "applemusic".to_string(),
            is_seekable: true,
            position: 0,
        })
    }

    async fn resolve_track(&self, id: &str) -> LoadResult {
        let path = format!("/catalog/{}/songs/{}", self.country_code, id);

        let data = match self.api_request(&path).await {
            Some(d) => d,
            None => return LoadResult::Empty {},
        };

        if let Some(item) = data.pointer("/data/0") {
            if let Some(info) = self.build_track(item, None) {
                return LoadResult::Track(Track::new(info));
            }
        }
        LoadResult::Empty {}
    }

    async fn resolve_album(&self, id: &str) -> LoadResult {
        let path = format!("/catalog/{}/albums/{}", self.country_code, id);
        let data = match self.api_request(&path).await {
            Some(d) => d,
            None => return LoadResult::Empty {},
        };

        let album = match data.pointer("/data/0") {
            Some(a) => a,
            None => return LoadResult::Empty {},
        };

        let name = album
            .pointer("/attributes/name")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown Album")
            .to_string();

        let artwork = album
            .pointer("/attributes/artwork/url")
            .and_then(|v| v.as_str())
            .map(|s| s.replace("{w}", "1000").replace("{h}", "1000"));

        let tracks_data = album
            .pointer("/relationships/tracks/data")
            .and_then(|v| v.as_array());

        let mut tracks = Vec::new();
        if let Some(items) = tracks_data {
            for item in items {
                if let Some(info) = self.build_track(item, artwork.clone()) {
                    tracks.push(Track::new(info));
                }
            }
        }

        if tracks.is_empty() {
            return LoadResult::Empty {};
        }

        LoadResult::Playlist(PlaylistData {
            info: PlaylistInfo {
                name,
                selected_track: -1,
            },
            plugin_info: serde_json::json!({}),
            tracks,
        })
    }

    async fn resolve_playlist(&self, id: &str) -> LoadResult {
        let path = format!("/catalog/{}/playlists/{}", self.country_code, id);
        let data = match self.api_request(&path).await {
            Some(d) => d,
            None => return LoadResult::Empty {},
        };

        let playlist = match data.pointer("/data/0") {
            Some(p) => p,
            None => return LoadResult::Empty {},
        };

        let name = playlist
            .pointer("/attributes/name")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown Playlist")
            .to_string();
        let artwork = playlist
            .pointer("/attributes/artwork/url")
            .and_then(|v| v.as_str())
            .map(|s| s.replace("{w}", "1000").replace("{h}", "1000"));

        let tracks_data = playlist
            .pointer("/relationships/tracks/data")
            .and_then(|v| v.as_array());

        let mut tracks = Vec::new();
        if let Some(items) = tracks_data {
            for item in items {
                if let Some(info) = self.build_track(item, artwork.clone()) {
                    tracks.push(Track::new(info));
                }
            }
        }

        if tracks.is_empty() {
            return LoadResult::Empty {};
        }

        LoadResult::Playlist(PlaylistData {
            info: PlaylistInfo {
                name,
                selected_track: -1,
            },
            plugin_info: serde_json::json!({}),
            tracks,
        })
    }

    async fn resolve_artist(&self, id: &str) -> LoadResult {
        // Fetch top songs
        let path = format!(
            "/catalog/{}/artists/{}/view/top-songs",
            self.country_code, id
        );
        let data = match self.api_request(&path).await {
            Some(d) => d,
            None => return LoadResult::Empty {},
        };

        let tracks_data = data.pointer("/data").and_then(|v| v.as_array());

        // Fetch artist info for name/artwork
        let artist_path = format!("/catalog/{}/artists/{}", self.country_code, id);
        let artist_data = self.api_request(&artist_path).await;

        let (artist_name, artwork) = if let Some(ad) = artist_data {
            let name = ad
                .pointer("/data/0/attributes/name")
                .and_then(|v| v.as_str())
                .unwrap_or("Artist")
                .to_string();
            let art = ad
                .pointer("/data/0/attributes/artwork/url")
                .and_then(|v| v.as_str())
                .map(|s| s.replace("{w}", "1000").replace("{h}", "1000"));
            (name, art)
        } else {
            ("Artist".to_string(), None)
        };

        let mut tracks = Vec::new();
        if let Some(items) = tracks_data {
            for item in items {
                if let Some(info) = self.build_track(item, artwork.clone()) {
                    tracks.push(Track::new(info));
                }
            }
        }

        if tracks.is_empty() {
            return LoadResult::Empty {};
        }

        LoadResult::Playlist(PlaylistData {
            info: PlaylistInfo {
                name: format!("{}'s Top Tracks", artist_name),
                selected_track: -1,
            },
            plugin_info: serde_json::json!({}),
            tracks,
        })
    }

    async fn search(&self, query: &str) -> LoadResult {
        // /catalog/{}/search?term={}&limit={}&types=songs
        let encoded_query = urlencoding::encode(query);
        let path = format!(
            "/catalog/{}/search?term={}&limit=10&types=songs",
            self.country_code, encoded_query
        );

        let data = match self.api_request(&path).await {
            Some(d) => d,
            None => return LoadResult::Empty {},
        };

        let songs = data
            .pointer("/results/songs/data")
            .and_then(|v| v.as_array());

        let mut tracks = Vec::new();
        if let Some(items) = songs {
            for item in items {
                if let Some(info) = self.build_track(item, None) {
                    tracks.push(Track::new(info));
                }
            }
        }

        if tracks.is_empty() {
            LoadResult::Empty {}
        } else {
            LoadResult::Search(tracks)
        }
    }
}
#[async_trait]
impl SourcePlugin for AppleMusicSource {
    fn name(&self) -> &str {
        "applemusic"
    }

    fn can_handle(&self, identifier: &str) -> bool {
        identifier.starts_with(&self.search_prefix) || self.url_regex.is_match(identifier)
    }

    async fn load(
        &self,
        identifier: &str,
        _routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> LoadResult {
        if identifier.starts_with(&self.search_prefix) {
            let query = &identifier[self.search_prefix.len()..];
            return self.search(query).await;
        }

        if let Some(caps) = self.url_regex.captures(identifier) {
            let type_str = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            let id = caps.get(2).map(|m| m.as_str()).unwrap_or("");
            let song_id = caps.get(3).map(|m| m.as_str()); // ?i=...

            // If it's an album link with ?i=..., treat it as a song
            if type_str == "album" && song_id.is_some() {
                return self.resolve_track(song_id.unwrap()).await;
            }

            match type_str {
                "song" => return self.resolve_track(id).await,
                "album" => return self.resolve_album(id).await,
                "playlist" => return self.resolve_playlist(id).await,
                "artist" => return self.resolve_artist(id).await,
                _ => return LoadResult::Empty {},
            }
        }

        LoadResult::Empty {}
    }
}
