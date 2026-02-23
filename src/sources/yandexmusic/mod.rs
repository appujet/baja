use std::sync::Arc;

use async_trait::async_trait;
use regex::Regex;
use serde_json::{Value, json};
use tracing::debug;

use crate::{
    api::tracks::{LoadResult, PlaylistData, PlaylistInfo, Track, TrackInfo},
    sources::{SourcePlugin, plugin::BoxedTrack},
};

pub mod track;
pub mod utils;

pub struct YandexMusicSource {
    client: reqwest::Client,
    #[allow(dead_code)]
    access_token: String,
    url_pattern: Regex,
    playlist_pattern: Regex,
    playlist_uuid_pattern: Regex,
    search_prefixes: Vec<String>,
    rec_prefixes: Vec<String>,
    search_limit: usize,
    playlist_load_limit: usize,
    album_load_limit: usize,
    artist_load_limit: usize,
    proxy: Option<crate::configs::HttpProxyConfig>,
}

const API_BASE: &str = "https://api.music.yandex.net";

impl YandexMusicSource {
    pub fn new(config: Option<crate::configs::YandexMusicConfig>) -> Result<Self, String> {
        let config = config.ok_or("Yandex Music configuration is missing")?;
        let access_token = config
            .access_token
            .ok_or("Yandex Music access token is missing")?;

        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::USER_AGENT,
            "Yandex-Music-API".parse().unwrap(),
        );
        headers.insert(
            "X-Yandex-Music-Client",
            "YandexMusicAndroid/24023621".parse().unwrap(),
        );
        headers.insert(
            reqwest::header::AUTHORIZATION,
            format!("OAuth {}", access_token).parse().unwrap(),
        );

        let mut client_builder = reqwest::Client::builder().default_headers(headers);

        if let Some(proxy_config) = &config.proxy {
            if let Some(url) = &proxy_config.url {
                debug!("Configuring proxy for YandexMusicSource: {}", url);
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

        let client = client_builder.build().map_err(|e| e.to_string())?;

        Ok(Self {
            client,
            access_token,
            url_pattern: Regex::new(r"(?i)^https?://music\.yandex\.(ru|com|kz|by)/(artist|album|track)/(?P<id1>[0-9]+)(/(track)/(?P<id2>[0-9]+))?/?").unwrap(),
            playlist_pattern: Regex::new(r"(?i)^https?://music\.yandex\.(ru|com|kz|by)/users/(?P<user>[^/]+)/playlists/(?P<id>[0-9]+)/?").unwrap(),
            playlist_uuid_pattern: Regex::new(r"(?i)^https?://music\.yandex\.(ru|com|kz|by)/playlists/(?P<uuid>[0-9a-z-]+)").unwrap(),
            search_prefixes: vec!["ymsearch:".to_string()],
            rec_prefixes: vec!["ymrec:".to_string()],
            search_limit: config.search_limit,
            playlist_load_limit: config.playlist_load_limit,
            album_load_limit: config.album_load_limit,
            artist_load_limit: config.artist_load_limit,
            proxy: config.proxy,
        })
    }

    async fn api_request(&self, endpoint: &str, params: Option<&[(&str, &str)]>) -> Option<Value> {
        let mut url = format!("{}{}", API_BASE, endpoint);
        if let Some(p) = params {
            let mut first = true;
            for (k, v) in p {
                url.push_str(if first { "?" } else { "&" });
                url.push_str(&format!("{}={}", k, urlencoding::encode(v)));
                first = false;
            }
        }
        debug!("Yandex Music API request: {}", url);
        let resp = self.client.get(&url).send().await.ok()?;
        let status = resp.status();
        debug!("Yandex Music API response status: {} -> {}", url, status);

        if !status.is_success() {
            debug!("Yandex Music API request failed: {} -> {}", url, status);
            return None;
        }

        let body: Value = resp.json().await.ok()?;
        debug!("Yandex Music API response body: {}", body);
        Some(body["result"].clone())
    }

    async fn search(&self, query: &str) -> LoadResult {
        let data = match self
            .api_request(
                "/search",
                Some(&[("text", query), ("type", "all"), ("page", "0")]),
            )
            .await
        {
            Some(d) => d,
            None => return LoadResult::Empty {},
        };

        let tracks = self.parse_tracks(&data["tracks"]["results"]);
        if tracks.is_empty() {
            return LoadResult::Empty {};
        }

        LoadResult::Search(tracks)
    }

    async fn load_search_internal(&self, query: &str) -> Option<crate::api::tracks::SearchResult> {
        let data = self
            .api_request(
                "/search",
                Some(&[("text", query), ("type", "all"), ("page", "0")]),
            )
            .await?;

        let tracks = self.parse_tracks(&data["tracks"]["results"]);

        let mut albums = Vec::new();
        if let Some(arr) = data["albums"]["results"].as_array() {
            for item in arr.iter().take(self.search_limit) {
                if let Some(playlist) = self.build_playlist_from_search(item, "album") {
                    albums.push(playlist);
                }
            }
        }

        let mut artists = Vec::new();
        if let Some(arr) = data["artists"]["results"].as_array() {
            for item in arr.iter().take(self.search_limit) {
                if let Some(playlist) = self.build_playlist_from_search(item, "artist") {
                    artists.push(playlist);
                }
            }
        }

        let mut playlists = Vec::new();
        if let Some(arr) = data["playlists"]["results"].as_array() {
            for item in arr.iter().take(self.search_limit) {
                if let Some(playlist) = self.build_playlist_from_search(item, "playlist") {
                    playlists.push(playlist);
                }
            }
        }

        Some(crate::api::tracks::SearchResult {
            tracks,
            albums,
            artists,
            playlists,
            texts: Vec::new(),
            plugin: json!({}),
        })
    }

    fn build_playlist_from_search(&self, item: &Value, r#type: &str) -> Option<PlaylistData> {
        if !item["available"].as_bool().unwrap_or(false) {
            return None;
        }

        let name = match r#type {
            "artist" => item["name"].as_str()?.to_string(),
            _ => item["title"].as_str()?.to_string(),
        };

        Some(PlaylistData {
            info: PlaylistInfo {
                name,
                selected_track: 0,
            },
            plugin_info: json!({ "type": r#type }),
            tracks: Vec::new(),
        })
    }

    async fn resolve_url(&self, url: &str) -> LoadResult {
        if let Some(caps) = self.playlist_pattern.captures(url) {
            let user = caps.name("user").map(|m| m.as_str()).unwrap();
            let id = caps.name("id").map(|m| m.as_str()).unwrap();
            return self.get_playlist(user, id).await;
        }
        if let Some(caps) = self.playlist_uuid_pattern.captures(url) {
            let uuid = caps.name("uuid").map(|m| m.as_str()).unwrap();
            return self.get_playlist_uuid(uuid).await;
        }
        if let Some(caps) = self.url_pattern.captures(url) {
            let type1 = caps.get(2).map(|m| m.as_str()).unwrap();
            let id1 = caps.name("id1").map(|m| m.as_str()).unwrap();

            match type1 {
                "track" => return self.get_track_internal(id1).await,
                "album" => {
                    if let Some(id2) = caps.name("id2") {
                        return self.get_track_internal(id2.as_str()).await;
                    }
                    return self.get_album(id1).await;
                }
                "artist" => return self.get_artist(id1).await,
                _ => {}
            }
        }

        LoadResult::Empty {}
    }

    async fn get_track_internal(&self, id: &str) -> LoadResult {
        let data = match self.api_request(&format!("/tracks/{}", id), None).await {
            Some(d) => d,
            None => return LoadResult::Empty {},
        };

        if let Some(track) = self.build_track(&data[0]) {
            LoadResult::Track(track)
        } else {
            LoadResult::Empty {}
        }
    }

    async fn get_album(&self, id: &str) -> LoadResult {
        let page_size = (self.album_load_limit * 50).max(50).to_string();
        let data = match self
            .api_request(
                &format!("/albums/{}/with-tracks", id),
                Some(&[("page-size", &page_size)]),
            )
            .await
        {
            Some(d) => d,
            None => return LoadResult::Empty {},
        };

        let mut tracks = Vec::new();
        if let Some(volumes) = data["volumes"].as_array() {
            for volume in volumes {
                if let Some(arr) = volume.as_array() {
                    for item in arr {
                        if let Some(track) = self.build_track(item) {
                            tracks.push(track);
                        }
                    }
                }
            }
        }

        if tracks.is_empty() {
            return LoadResult::Empty {};
        }

        LoadResult::Playlist(PlaylistData {
            info: PlaylistInfo {
                name: data["title"]
                    .as_str()
                    .unwrap_or("Yandex Music Album")
                    .to_string(),
                selected_track: 0,
            },
            plugin_info: json!({ "type": "album" }),
            tracks,
        })
    }

    async fn get_artist(&self, id: &str) -> LoadResult {
        let page_size = (self.artist_load_limit * 10).max(10).to_string();
        let data = match self
            .api_request(
                &format!("/artists/{}/tracks", id),
                Some(&[("page-size", &page_size)]),
            )
            .await
        {
            Some(d) => d,
            None => return LoadResult::Empty {},
        };

        let tracks = self.parse_tracks(&data["tracks"]);
        if tracks.is_empty() {
            return LoadResult::Empty {};
        }

        let artist_data = match self.api_request(&format!("/artists/{}", id), None).await {
            Some(d) => d,
            None => return LoadResult::Empty {},
        };

        let name = artist_data["artist"]["name"].as_str().unwrap_or("Artist");

        LoadResult::Playlist(PlaylistData {
            info: PlaylistInfo {
                name: format!("{}'s Top Tracks", name),
                selected_track: 0,
            },
            plugin_info: json!({ "type": "artist" }),
            tracks,
        })
    }

    async fn get_playlist(&self, user: &str, id: &str) -> LoadResult {
        let page_size = (self.playlist_load_limit * 100).max(100).to_string();
        let data = match self
            .api_request(
                &format!("/users/{}/playlists/{}", user, id),
                Some(&[("page-size", &page_size), ("rich-tracks", "true")]),
            )
            .await
        {
            Some(d) => d,
            None => return LoadResult::Empty {},
        };

        self.build_playlist_result(data)
    }

    async fn get_playlist_uuid(&self, uuid: &str) -> LoadResult {
        let page_size = (self.playlist_load_limit * 100).max(100).to_string();
        let data = match self
            .api_request(
                &format!("/playlist/{}", uuid),
                Some(&[("page-size", &page_size), ("rich-tracks", "true")]),
            )
            .await
        {
            Some(d) => d,
            None => return LoadResult::Empty {},
        };

        self.build_playlist_result(data)
    }

    fn build_playlist_result(&self, data: Value) -> LoadResult {
        let tracks = self.parse_tracks(&data["tracks"]);
        if tracks.is_empty() {
            return LoadResult::Empty {};
        }

        let title = if data["kind"].as_u64() == Some(3) {
            let owner = data["owner"]["name"]
                .as_str()
                .or(data["owner"]["login"].as_str())
                .unwrap_or("User");
            format!("{}'s liked songs", owner)
        } else {
            data["title"]
                .as_str()
                .unwrap_or("Yandex Music Playlist")
                .to_string()
        };

        LoadResult::Playlist(PlaylistData {
            info: PlaylistInfo {
                name: title,
                selected_track: 0,
            },
            plugin_info: json!({ "type": "playlist" }),
            tracks,
        })
    }

    fn parse_tracks(&self, data: &Value) -> Vec<Track> {
        let mut tracks = Vec::new();
        if let Some(arr) = data.as_array() {
            for item in arr {
                let track_json = if item.get("track").is_some() {
                    &item["track"]
                } else {
                    item
                };
                if let Some(track) = self.build_track(track_json) {
                    tracks.push(track);
                }
            }
        }
        tracks
    }

    fn build_track(&self, data: &Value) -> Option<Track> {
        if !data["available"].as_bool().unwrap_or(false) {
            return None;
        }

        let id = data["id"]
            .as_u64()
            .map(|n| n.to_string())
            .or(data["id"].as_str().map(|s| s.to_string()))?;
        let title = data["title"].as_str()?;
        let author = self.parse_artist(data);
        let duration = data["durationMs"].as_u64().unwrap_or(0);
        let uri = Some(format!("https://music.yandex.ru/track/{}", id));
        let artwork_url = self.parse_cover_uri(data);

        Some(Track::new(TrackInfo {
            identifier: id,
            is_seekable: true,
            author,
            length: duration,
            is_stream: false,
            position: 0,
            title: title.to_string(),
            uri,
            artwork_url,
            isrc: data["isrc"].as_str().map(|s| s.to_string()),
            source_name: "yandexmusic".to_string(),
        }))
    }

    fn parse_artist(&self, data: &Value) -> String {
        if let Some(arr) = data["artists"].as_array() {
            return arr
                .iter()
                .filter_map(|a| a["name"].as_str())
                .collect::<Vec<_>>()
                .join(", ");
        }
        "Unknown Artist".to_string()
    }

    fn parse_cover_uri(&self, data: &Value) -> Option<String> {
        let uri = data["ogImage"]
            .as_str()
            .or(data["coverUri"].as_str())
            .or(data["cover"]["uri"].as_str())?;

        Some(format!("https://{}", uri.replace("%%", "400x400")))
    }
}

#[async_trait]
impl SourcePlugin for YandexMusicSource {
    fn name(&self) -> &str {
        "yandexmusic"
    }

    fn can_handle(&self, identifier: &str) -> bool {
        self.search_prefixes
            .iter()
            .any(|p| identifier.starts_with(p))
            || self.rec_prefixes.iter().any(|p| identifier.starts_with(p))
            || self.url_pattern.is_match(identifier)
            || self.playlist_pattern.is_match(identifier)
            || self.playlist_uuid_pattern.is_match(identifier)
    }

    fn search_prefixes(&self) -> Vec<&str> {
        self.search_prefixes.iter().map(|s| s.as_str()).collect()
    }

    fn rec_prefixes(&self) -> Vec<&str> {
        self.rec_prefixes.iter().map(|s| s.as_str()).collect()
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
            return self.search(identifier.strip_prefix(prefix).unwrap()).await;
        }

        if let Some(prefix) = self
            .rec_prefixes
            .iter()
            .find(|p| identifier.starts_with(*p))
        {
            // For now, ymrec: query maps to search as Yandex recommendations
            // usually require a track/artist ID which search helps find.
            return self.search(identifier.strip_prefix(prefix).unwrap()).await;
        }

        self.resolve_url(identifier).await
    }

    async fn load_search(
        &self,
        query: &str,
        _types: &[String],
        _routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<crate::api::tracks::SearchResult> {
        self.load_search_internal(query).await
    }

    async fn get_track(
        &self,
        identifier: &str,
        routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<BoxedTrack> {
        let track_id = if identifier.starts_with("http") {
            if let Some(caps) = self.url_pattern.captures(identifier) {
                caps.name("id2")
                    .or(caps.name("id1"))
                    .map(|m| m.as_str().to_string())?
            } else {
                return None;
            }
        } else {
            identifier.to_string()
        };

        Some(Box::new(track::YandexMusicTrack {
            client: self.client.clone(),
            track_id,
            local_addr: routeplanner.and_then(|rp| rp.get_address()),
            proxy: self.proxy.clone(),
        }))
    }

    fn get_proxy_config(&self) -> Option<crate::configs::HttpProxyConfig> {
        self.proxy.clone()
    }
}
