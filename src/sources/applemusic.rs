use crate::api::tracks::{LoadResult, PlaylistData, PlaylistInfo, Track, TrackInfo};
use crate::sources::SourcePlugin;
use async_trait::async_trait;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use regex::Regex;
use reqwest::header::{HeaderMap, HeaderValue, USER_AGENT};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

const API_BASE: &str = "https://api.music.apple.com/v1";

#[derive(Clone, Debug)]
struct AppleMusicToken {
    access_token: String,
    origin: Option<String>,
    expiry_ms: u64,
}

struct AppleMusicTokenTracker {
    token: RwLock<Option<AppleMusicToken>>,
    client: reqwest::Client,
}

impl AppleMusicTokenTracker {
    fn new(client: reqwest::Client) -> Self {
        Self {
            token: RwLock::new(None),
            client,
        }
    }

    async fn get_token(&self) -> Option<String> {
        {
            let lock = self.token.read().await;
            if let Some(token) = &*lock {
                if self.is_valid(token) {
                    return Some(token.access_token.clone());
                }
            }
        }
        self.refresh_token().await
    }

    async fn get_origin(&self) -> Option<String> {
        let lock = self.token.read().await;
        lock.as_ref().and_then(|t| t.origin.clone())
    }

    fn is_valid(&self, token: &AppleMusicToken) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        token.expiry_ms > now + 10_000
    }

    async fn refresh_token(&self) -> Option<String> {
        info!("Fetching new Apple Music API token...");
        
        let browse_url = "https://music.apple.com";
        let resp = match self.client.get(browse_url).send().await {
            Ok(r) => r,
            Err(e) => {
                error!("Failed to fetch Apple Music root page: {}", e);
                return None;
            }
        };

        if !resp.status().is_success() {
             error!("Apple Music root page returned status: {}", resp.status());
             return None;
        }

        let html = resp.text().await.unwrap_or_default();
        
        let script_regex = Regex::new(r#"<script\s+type="module"\s+crossorigin\s+src="(/assets/index[^"]+\.js)""#).unwrap();
        let script_path = match script_regex.captures(&html) {
            Some(caps) => caps.get(1)?.as_str(),
            None => {
                let index_regex = Regex::new(r#"/assets/index[^"]+\.js"#).unwrap();
                match index_regex.find(&html) {
                    Some(m) => m.as_str(),
                    None => {
                        error!("Could not find index JS in Apple Music HTML");
                        return None;
                    }
                }
            }
        };

        let script_url = if script_path.starts_with("http") {
             script_path.to_string()
        } else {
             format!("https://music.apple.com{}", script_path)
        };

        let js_resp = match self.client.get(&script_url).send().await {
            Ok(r) => r,
            Err(e) => {
                error!("Failed to fetch Apple Music JS bundle: {}", e);
                return None;
            }
        };

        let js_content = js_resp.text().await.unwrap_or_default();

        let token_regex = Regex::new(r#"(ey[\w-]+\.[\w-]+\.[\w-]+)"#).unwrap();
        let token_str = match token_regex.find(&js_content) {
            Some(m) => m.as_str().to_string(),
            None => {
                error!("Could not find bearer token in Apple Music JS");
                return None;
            }
        };

        let (origin, expiry_ms) = self.parse_jwt(&token_str).unwrap_or((None, 0));

        let token = AppleMusicToken {
            access_token: token_str.clone(),
            origin,
            expiry_ms,
        };

        let mut lock = self.token.write().await;
        *lock = Some(token);

        info!("Successfully refreshed Apple Music token");
        Some(token_str)
    }

    fn parse_jwt(&self, token: &str) -> Option<(Option<String>, u64)> {
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() < 2 { return None; }

        let payload_part = parts[1];
        let decoded = match URL_SAFE_NO_PAD.decode(payload_part) {
            Ok(d) => d,
            Err(_) => return None,
        };

        let json_str = String::from_utf8(decoded).ok()?;
        let json: Value = serde_json::from_str(&json_str).ok()?;

        let origin = json.get("root_https_origin")
            .and_then(|v| v.as_array())
            .and_then(|a| a.first())
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let exp = json.get("exp").and_then(|v| v.as_u64()).map(|e| e * 1000).unwrap_or(0);

        Some((origin, exp))
    }
}

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
            token_tracker: Arc::new(AppleMusicTokenTracker::new(client.clone())),
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
        let title = attributes.get("name").and_then(|v| v.as_str()).unwrap_or("Unknown Title").to_string();
        let author = attributes.get("artistName").and_then(|v| v.as_str()).unwrap_or("Unknown Artist").to_string();
        let length = attributes.get("durationInMillis").and_then(|v| v.as_u64()).unwrap_or(0);
        
        let isrc = attributes.get("isrc").and_then(|v| v.as_str()).map(|s| s.to_string());
        
        let artwork_url = artwork_override.or_else(|| {
             attributes.get("artwork")
                .and_then(|a| a.get("url"))
                .and_then(|v| v.as_str())
                .map(|s| s.replace("{w}", "1000").replace("{h}", "1000"))
        });

        let url = attributes.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string();

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

        let name = album.pointer("/attributes/name").and_then(|v| v.as_str()).unwrap_or("Unknown Album").to_string();
        
        let artwork = album.pointer("/attributes/artwork/url")
             .and_then(|v| v.as_str())
             .map(|s| s.replace("{w}", "1000").replace("{h}", "1000"));

        let tracks_data = album.pointer("/relationships/tracks/data").and_then(|v| v.as_array());
        
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

        let name = playlist.pointer("/attributes/name").and_then(|v| v.as_str()).unwrap_or("Unknown Playlist").to_string();
        let artwork = playlist.pointer("/attributes/artwork/url")
             .and_then(|v| v.as_str())
             .map(|s| s.replace("{w}", "1000").replace("{h}", "1000"));

        let tracks_data = playlist.pointer("/relationships/tracks/data").and_then(|v| v.as_array());
         
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
        let path = format!("/catalog/{}/artists/{}/view/top-songs", self.country_code, id);
        let data = match self.api_request(&path).await {
             Some(d) => d,
             None => return LoadResult::Empty {},
        };
        
        let tracks_data = data.pointer("/data").and_then(|v| v.as_array());
         
         // Fetch artist info for name/artwork
        let artist_path = format!("/catalog/{}/artists/{}", self.country_code, id);
        let artist_data = self.api_request(&artist_path).await;
        
        let (artist_name, artwork) = if let Some(ad) = artist_data {
             let name = ad.pointer("/data/0/attributes/name").and_then(|v| v.as_str()).unwrap_or("Artist").to_string();
             let art = ad.pointer("/data/0/attributes/artwork/url")
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
         let path = format!("/catalog/{}/search?term={}&limit=10&types=songs", self.country_code, encoded_query);
         
         let data = match self.api_request(&path).await {
             Some(d) => d,
             None => return LoadResult::Empty {},
         };
         
         let songs = data.pointer("/results/songs/data").and_then(|v| v.as_array());
         
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

    async fn load(&self, identifier: &str, _routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>) -> LoadResult {
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

    async fn get_playback_url(&self, _identifier: &str, _routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>) -> Option<String> {
        None // Mirror source, no playback URL
    }
}
