use async_trait::async_trait;
use regex::Regex;
use reqwest::header::HeaderMap;
use serde_json::Value;
use std::sync::Arc;
use tracing::debug;

use super::{token::DeezerTokenTracker, track::DeezerTrack};
use crate::{
    api::tracks::{LoadResult, PlaylistData, PlaylistInfo, Track, TrackInfo},
    sources::{SourcePlugin, plugin::PlayableTrack},
};

const PUBLIC_API_BASE: &str = "https://api.deezer.com/2.0";
const PRIVATE_API_BASE: &str = "https://www.deezer.com/ajax/gw-light.php";

pub struct DeezerSource {
    client: reqwest::Client,
    config: crate::configs::DeezerConfig,
    pub token_tracker: Arc<DeezerTokenTracker>,
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
        let mut arls = config.arls.clone().unwrap_or_default();
        arls.retain(|s| !s.is_empty());
        arls.sort();
        arls.dedup();

        if arls.is_empty() {
            return Err("Deezer arls must be set".to_string());
        }

        let mut headers = HeaderMap::new();
        headers.insert("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/115.0.0.0 Safari/537.36".parse().unwrap());

        let mut client_builder = reqwest::Client::builder()
            .default_headers(headers)
            .cookie_store(true);

        if let Some(proxy_config) = &config.proxy {
            if let Some(url) = &proxy_config.url {
                debug!("Configuring proxy for DeezerSource: {}", url);
                if let Ok(mut proxy_obj) = reqwest::Proxy::all(url) {
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
        let token_tracker = Arc::new(DeezerTokenTracker::new(client.clone(), arls));

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
        self.client.get(&url).send().await.ok()?.json().await.ok()
    }

    fn parse_track(&self, json: &Value) -> Option<Track> {
        let id = json.get("id")?.to_string();
        let title = json.get("title")?.as_str()?.to_string();
        let artist = json.get("artist")?.get("name")?.as_str()?.to_string();
        let duration = json.get("duration")?.as_u64()? * 1000;
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

        Some(Track::new(TrackInfo {
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
        }))
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
        let json = self.get_json_public(&url).await?;
        if json.get("id").is_some() {
            self.parse_track(&json)
        } else {
            None
        }
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
            let track_id = query.strip_prefix(&self.rec_track_prefix).unwrap_or(query);
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
            Err(_) => return LoadResult::Empty {},
        };

        let json: Value = res.json().await.unwrap_or(Value::Null);
        let data = json.get("results").and_then(|r| r.get("data"));

        let tracks: Vec<Track> = if let Some(arr) = data.and_then(|d| d.as_array()) {
            arr.iter()
                .filter_map(|item| self.parse_recommendation_track(item))
                .collect()
        } else if let Some(obj) = data.and_then(|d| d.as_object()) {
            obj.values()
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
            plugin_info: Value::Null,
            tracks,
        })
    }

    fn parse_recommendation_track(&self, json: &Value) -> Option<Track> {
        let id = json
            .get("SNG_ID")?
            .as_str()
            .map(|s| s.to_string())
            .or_else(|| json.get("SNG_ID").map(|v| v.to_string()))?;
        let title = json.get("SNG_TITLE")?.as_str()?.to_string();
        let artist = json.get("ART_NAME")?.as_str()?.to_string();
        let duration = json.get("DURATION")?.as_u64()? * 1000;
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

        Some(Track::new(TrackInfo {
            identifier: id.clone(),
            is_seekable: true,
            author: artist,
            length: duration,
            is_stream: false,
            position: 0,
            title,
            uri: Some(format!("https://deezer.com/track/{}", id)),
            artwork_url,
            isrc,
            source_name: "deezer".to_string(),
        }))
    }

    async fn get_album(&self, id: &str) -> LoadResult {
        let json = match self.get_json_public(&format!("album/{}", id)).await {
            Some(j) => j,
            None => return LoadResult::Empty {},
        };
        let tracks_json = match self
            .get_json_public(&format!("album/{}/tracks?limit=10000", id))
            .await
        {
            Some(j) => j,
            None => return LoadResult::Empty {},
        };
        let mut tracks = Vec::new();
        let artwork_url = json
            .get("cover_xl")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
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
            plugin_info: Value::Null,
            tracks,
        })
    }

    async fn get_playlist(&self, id: &str) -> LoadResult {
        let json = match self.get_json_public(&format!("playlist/{}", id)).await {
            Some(j) => j,
            None => return LoadResult::Empty {},
        };
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
            plugin_info: Value::Null,
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
            plugin_info: Value::Null,
            tracks,
        })
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
            return self
                .search(identifier.strip_prefix(&self.search_prefix).unwrap())
                .await;
        }
        if identifier.starts_with(&self.isrc_prefix) {
            if let Some(track) = self
                .get_track_by_isrc(identifier.strip_prefix(&self.isrc_prefix).unwrap())
                .await
            {
                return LoadResult::Track(track);
            }
            return LoadResult::Empty {};
        }
        if identifier.starts_with(&self.rec_prefix) {
            return self
                .get_recommendations(identifier.strip_prefix(&self.rec_prefix).unwrap())
                .await;
        }
        if identifier.starts_with(&self.share_url_prefix) {
            let client = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap_or_else(|_| self.client.clone());
            if let Ok(res) = client.get(identifier).send().await {
                if res.status().is_redirection() {
                    if let Some(loc) = res.headers().get("location").and_then(|l| l.to_str().ok()) {
                        if loc.starts_with("https://www.deezer.com/") {
                            return self.load(loc, routeplanner).await;
                        }
                    }
                }
            }
            return LoadResult::Empty {};
        }
        if let Some(caps) = self.url_regex.captures(identifier) {
            let type_ = caps.name("type").map(|m| m.as_str()).unwrap_or("");
            let id = caps.name("id").map(|m| m.as_str()).unwrap_or("");
            return match type_ {
                "track" => {
                    if let Some(json) = self.get_json_public(&format!("track/{}", id)).await {
                        if let Some(track) = self.parse_track(&json) {
                            return LoadResult::Track(track);
                        }
                    }
                    LoadResult::Empty {}
                }
                "album" => self.get_album(id).await,
                "playlist" => self.get_playlist(id).await,
                "artist" => self.get_artist(id).await,
                _ => LoadResult::Empty {},
            };
        }
        LoadResult::Empty {}
    }

    async fn get_track(
        &self,
        identifier: &str,
        routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<Box<dyn PlayableTrack>> {
        let track_id = if let Some(caps) = self.url_regex.captures(identifier) {
            caps.name("id").map(|m| m.as_str())?.to_string()
        } else {
            identifier.to_string()
        };

        Some(Box::new(DeezerTrack {
            client: self.client.clone(),
            track_id,
            arl_index: 0, // get_token will rotate
            token_tracker: self.token_tracker.clone(),
            master_key: self
                .config
                .master_decryption_key
                .clone()
                .unwrap_or_default(),
            local_addr: routeplanner.and_then(|rp| rp.get_address()),
            proxy: self.config.proxy.clone(),
        }))
    }

    fn get_proxy_config(&self) -> Option<crate::configs::HttpProxyConfig> {
        self.config.proxy.clone()
    }
}
