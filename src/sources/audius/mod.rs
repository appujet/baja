use std::sync::Arc;

use async_trait::async_trait;
use regex::Regex;
use serde_json::{Value, json};

use crate::{
    protocol::tracks::{LoadResult, PlaylistData, PlaylistInfo, Track, TrackInfo},
    sources::{BoxedTrack, SourcePlugin, StreamInfo},
};

pub mod track;

pub struct AudiusSource {
    client: Arc<reqwest::Client>,
    track_pattern: Regex,
    playlist_pattern: Regex,
    album_pattern: Regex,
    user_pattern: Regex,
    search_prefixes: Vec<String>,
    app_name: String,
    search_limit: usize,
    playlist_load_limit: usize,
    album_load_limit: usize,
}

impl AudiusSource {
    pub fn new(
        config: Option<crate::configs::AudiusConfig>,
        client: Arc<reqwest::Client>,
    ) -> Result<Self, String> {
        let config = config.unwrap_or_default();

        Ok(Self {
            client,
            track_pattern: Regex::new(r"(?i)^https?://(?:www\.)?audius\.co/(?P<artist>[^/]+)/(?P<slug>[^/?#]+)(?:\?.*)?$").unwrap(),
            playlist_pattern: Regex::new(r"(?i)^https?://(?:www\.)?audius\.co/(?P<artist>[^/]+)/playlist/(?P<slug>[^/?#]+)(?:\?.*)?$").unwrap(),
            album_pattern: Regex::new(r"(?i)^https?://(?:www\.)?audius\.co/(?P<artist>[^/]+)/album/(?P<slug>[^/?#]+)(?:\?.*)?$").unwrap(),
            user_pattern: Regex::new(r"(?i)^https?://(?:www\.)?audius\.co/(?P<user>[^/?#]+)(?:\?.*)?$").unwrap(),
            search_prefixes: vec!["ausearch:".to_string(), "audsearch:".to_string()],
            app_name: config.app_name.unwrap_or_else(|| "Rustalink".to_string()),
            search_limit: config.search_limit,
            playlist_load_limit: config.playlist_load_limit,
            album_load_limit: config.album_load_limit,
        })
    }

    async fn api_request(&self, endpoint: &str) -> Option<Value> {
        let url = format!("https://discoveryprovider.audius.co{}", endpoint);
        let mut url_obj = reqwest::Url::parse(&url).ok()?;
        url_obj
            .query_pairs_mut()
            .append_pair("app_name", &self.app_name);

        let resp = self.client.get(url_obj).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }

        let body: Value = resp.json().await.ok()?;
        Some(body["data"].clone())
    }

    async fn search(&self, query: &str) -> LoadResult {
        let endpoint = format!(
            "/v1/tracks/search?query={}&limit={}",
            urlencoding::encode(query),
            self.search_limit
        );

        let data = match self.api_request(&endpoint).await {
            Some(d) => d,
            None => return LoadResult::Empty {},
        };

        let tracks = self.parse_tracks(&data);
        if tracks.is_empty() {
            return LoadResult::Empty {};
        }

        LoadResult::Search(tracks)
    }

    async fn resolve_url(&self, url: &str) -> LoadResult {
        if self.playlist_pattern.is_match(url) {
            return self.resolve_playlist_or_album(url, "playlist").await;
        }
        if self.album_pattern.is_match(url) {
            return self.resolve_playlist_or_album(url, "album").await;
        }
        if self.track_pattern.is_match(url) {
            return self.resolve_track(url).await;
        }
        if self.user_pattern.is_match(url) {
            return self.resolve_user(url).await;
        }

        LoadResult::Empty {}
    }

    async fn resolve_track(&self, url: &str) -> LoadResult {
        let endpoint = format!("/v1/resolve?url={}", urlencoding::encode(url));
        let data = match self.api_request(&endpoint).await {
            Some(d) => d,
            None => return LoadResult::Empty {},
        };

        if let Some(track) = self.build_track(&data) {
            LoadResult::Track(track)
        } else {
            LoadResult::Empty {}
        }
    }

    async fn resolve_playlist_or_album(&self, url: &str, _type: &str) -> LoadResult {
        let endpoint = format!("/v1/resolve?url={}", urlencoding::encode(url));
        let data = match self.api_request(&endpoint).await {
            Some(d) => d,
            None => return LoadResult::Empty {},
        };

        let id = match data["id"].as_str() {
            Some(i) => i,
            None => return LoadResult::Empty {},
        };

        let limit = if _type == "playlist" {
            self.playlist_load_limit
        } else {
            self.album_load_limit
        };
        let tracks_endpoint = format!("/v1/playlists/{}/tracks?limit={}", id, limit);
        let tracks_data = match self.api_request(&tracks_endpoint).await {
            Some(d) => d,
            None => return LoadResult::Empty {},
        };

        let tracks = self.parse_tracks(&tracks_data);
        if tracks.is_empty() {
            return LoadResult::Empty {};
        }

        let name = data["playlist_name"]
            .as_str()
            .unwrap_or("Audius Playlist")
            .to_string();

        LoadResult::Playlist(PlaylistData {
            info: PlaylistInfo {
                name,
                selected_track: 0,
            },
            plugin_info: json!({}),
            tracks,
        })
    }

    async fn resolve_user(&self, url: &str) -> LoadResult {
        let endpoint = format!("/v1/resolve?url={}", urlencoding::encode(url));
        let data = match self.api_request(&endpoint).await {
            Some(d) => d,
            None => return LoadResult::Empty {},
        };

        let id = match data["id"].as_str() {
            Some(i) => i,
            None => return LoadResult::Empty {},
        };

        let tracks_endpoint = format!("/v1/users/{}/tracks?limit={}", id, self.search_limit);
        let tracks_data = match self.api_request(&tracks_endpoint).await {
            Some(d) => d,
            None => return LoadResult::Empty {},
        };

        let tracks = self.parse_tracks(&tracks_data);
        if tracks.is_empty() {
            return LoadResult::Empty {};
        }

        let name = format!("{}'s Tracks", data["name"].as_str().unwrap_or("Artist"));

        LoadResult::Playlist(PlaylistData {
            info: PlaylistInfo {
                name,
                selected_track: 0,
            },
            plugin_info: json!({}),
            tracks,
        })
    }

    fn parse_tracks(&self, data: &Value) -> Vec<Track> {
        let mut tracks = Vec::new();
        if let Some(arr) = data.as_array() {
            for item in arr {
                if let Some(track) = self.build_track(item) {
                    tracks.push(track);
                }
            }
        }
        tracks
    }

    fn build_track(&self, data: &Value) -> Option<Track> {
        let id = data["id"].as_str()?;
        let title = data["title"].as_str()?;
        let author = data["user"]["name"].as_str().unwrap_or("Unknown Artist");
        let duration = (data["duration"].as_f64().unwrap_or(0.0) * 1000.0) as u64;
        let uri = data["permalink"].as_str().map(|p| {
            if p.starts_with("http") {
                p.to_string()
            } else {
                format!("https://audius.co{}", p)
            }
        });
        let artwork_url = self.get_artwork_url(&data["artwork"]);

        Some(Track::new(TrackInfo {
            identifier: id.to_string(),
            is_seekable: true,
            author: author.to_string(),
            length: duration,
            is_stream: false,
            position: 0,
            title: title.to_string(),
            uri,
            artwork_url,
            isrc: None,
            source_name: "audius".to_string(),
        }))
    }

    fn get_artwork_url(&self, artwork: &Value) -> Option<String> {
        if artwork.is_null() {
            return None;
        }

        if let Some(url) = artwork.as_str() {
            return Some(if url.starts_with("/") {
                format!("https://audius.co{}", url)
            } else {
                url.to_string()
            });
        }

        for size in &["480x480", "1000x1000", "150x150"] {
            if let Some(url) = artwork[size].as_str() {
                return Some(if url.starts_with("/") {
                    format!("https://audius.co{}", url)
                } else {
                    url.to_string()
                });
            }
        }
        None
    }
}

#[async_trait]
impl SourcePlugin for AudiusSource {
    fn name(&self) -> &str {
        "audius"
    }

    fn can_handle(&self, identifier: &str) -> bool {
        self.search_prefixes
            .iter()
            .any(|p| identifier.starts_with(p))
            || self.track_pattern.is_match(identifier)
            || self.playlist_pattern.is_match(identifier)
            || self.album_pattern.is_match(identifier)
            || self.user_pattern.is_match(identifier)
    }

    fn search_prefixes(&self) -> Vec<&str> {
        self.search_prefixes.iter().map(|s| s.as_str()).collect()
    }

    async fn load(
        &self,
        identifier: &str,
        _routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> LoadResult {
        for prefix in &self.search_prefixes {
            if identifier.starts_with(prefix) {
                return self.search(&identifier[prefix.len()..]).await;
            }
        }

        self.resolve_url(identifier).await
    }

    async fn get_track(
        &self,
        _identifier: &str,
        routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<BoxedTrack> {
        let track_id = if _identifier.starts_with("http") {
            let endpoint = format!("/v1/resolve?url={}", urlencoding::encode(_identifier));
            let data = self.api_request(&endpoint).await?;
            data["id"].as_str()?.to_string()
        } else {
            _identifier.to_string()
        };

        let stream_url = track::fetch_stream_url(&self.client, &track_id, &self.app_name).await?;

        Some(Box::new(track::AudiusTrack {
            client: self.client.clone(),
            track_id,
            stream_url: Some(stream_url),
            app_name: self.app_name.clone(),
            local_addr: routeplanner.and_then(|rp| rp.get_address()),
        }))
    }

    async fn get_stream_url(&self, identifier: &str, _itag: Option<i64>) -> Option<StreamInfo> {
        let track_id = if identifier.starts_with("http") {
            let endpoint = format!("/v1/resolve?url={}", urlencoding::encode(identifier));
            let data = self.api_request(&endpoint).await?;
            data["id"].as_str()?.to_string()
        } else {
            identifier.to_string()
        };
        let url = track::fetch_stream_url(&self.client, &track_id, &self.app_name).await?;
        Some(StreamInfo {
            url,
            mime_type: "audio/mpeg".to_string(),
            protocol: "http".to_string(),
        })
    }
}
