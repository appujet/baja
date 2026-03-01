use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};
use tracing::{debug, error};

use crate::{
    common::types::AnyResult,
    configs::Config,
    protocol::tracks::{LoadResult, PlaylistData, PlaylistInfo, Track, TrackInfo},
    sources::{
        SourcePlugin,
        plugin::BoxedTrack,
        qobuz::{token::QobuzTokenTracker, track::QobuzTrack},
    },
};

const API_URL: &str = "https://www.qobuz.com/api.json/0.2/";

pub struct QobuzSource {
    client: Arc<reqwest::Client>,
    token_tracker: Arc<QobuzTokenTracker>,
    search_limit: usize,
    album_load_limit: usize,
    playlist_load_limit: usize,
    artist_load_limit: usize,
    url_regex: regex::Regex,
    search_prefixes: Vec<String>,
    isrc_prefixes: Vec<String>,
    rec_prefixes: Vec<String>,
}

impl QobuzSource {
    pub fn new(config: &Config, client: Arc<reqwest::Client>) -> Result<Self, String> {
        let qobuz_config = config.qobuz.clone().unwrap_or_default();

        let tracker = Arc::new(QobuzTokenTracker::new(
            client.clone(),
            qobuz_config.user_token,
            qobuz_config.app_id,
            qobuz_config.app_secret,
        ));

        tracker.clone().init();

        Ok(Self {
            client,
            token_tracker: tracker,
            search_limit: qobuz_config.search_limit,
            album_load_limit: qobuz_config.album_load_limit,
            playlist_load_limit: qobuz_config.playlist_load_limit,
            artist_load_limit: qobuz_config.artist_load_limit,
            url_regex: regex::Regex::new(
                r"https?://(?:www\.|play\.|open\.)?qobuz\.com/(?:(?:[a-z]{2}-[a-z]{2}/)?(?P<type>album|playlist|track|artist)/(?:.+?/)?(?P<id>[a-zA-Z0-9]+)|(?P<type2>playlist)/(?P<id2>\d+))"
            ).unwrap(),
            search_prefixes: vec!["qbsearch:".to_string()],
            isrc_prefixes: vec!["qbisrc:".to_string()],
            rec_prefixes: vec!["qbrec:".to_string()],
        })
    }

    pub async fn api_request(&self, path: &str, params: Vec<(&str, String)>) -> AnyResult<Value> {
        let tokens = self
            .token_tracker
            .get_tokens()
            .await
            .ok_or_else(|| "Failed to get Qobuz tokens".to_string())?;

        let mut url = reqwest::Url::parse(&(API_URL.to_string() + path))?;
        {
            let mut query = url.query_pairs_mut();
            for (k, v) in params {
                query.append_pair(k, &v);
            }
        }

        let mut request = self
            .base_request(self.client.get(url))
            .header("Accept", "application/json")
            .header("x-app-id", &tokens.app_id);

        if let Some(user_token) = &tokens.user_token {
            request = request.header("x-user-auth-token", user_token);
        }

        let resp = request.send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("Qobuz API error ({}): {}", status, body).into());
        }

        let json: Value = resp.json().await?;
        Ok(json)
    }

    pub fn base_request(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        builder.header(reqwest::header::USER_AGENT, "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.0.0 Safari/537.36")
    }

    pub fn parse_qobuz_track(&self, json: &Value) -> QobuzTrack {
        let identifier = json["id"]
            .as_i64()
            .or_else(|| json["id"].as_str().and_then(|s| s.parse::<i64>().ok()))
            .unwrap_or(0)
            .to_string();

        let title = json["title"]
            .as_str()
            .unwrap_or("Unknown Title")
            .to_string();

        let (author, artist_url) = if !json["artist"].is_null() && json["artist"].is_object() {
            let name = json["artist"]["name"]["display"]
                .as_str()
                .or_else(|| json["artist"]["name"].as_str())
                .unwrap_or("Unknown Artist")
                .to_string();
            let url = json["artist"]["id"]
                .as_i64()
                .map(|id| format!("https://open.qobuz.com/artist/{}", id));
            (name, url)
        } else {
            let name = json["album"]["artist"]["name"]
                .as_str()
                .unwrap_or("Unknown Artist")
                .to_string();
            let url = json["album"]["artist"]["id"]
                .as_i64()
                .map(|id| format!("https://open.qobuz.com/artist/{}", id));
            (name, url)
        };

        let length = json["duration"].as_i64().unwrap_or(0) * 1000;
        let artwork_url = json["album"]["image"]["large"]
            .as_str()
            .map(|s| s.to_string());

        let isrc = json["isrc"].as_str().map(|s| s.to_string());
        let uri = format!("https://open.qobuz.com/track/{}", identifier);

        let album_name = json["album"]["title"].as_str().map(|s| s.to_string());
        let album_url = json["album"]["id"]
            .as_i64()
            .map(|id| format!("https://open.qobuz.com/album/{}", id));

        let artist_artwork_url =
            if !json["album"]["artist"].is_null() && !json["album"]["artist"]["image"].is_null() {
                json["album"]["artist"]["image"]
                    .as_str()
                    .map(|s| s.to_string())
            } else {
                None
            };

        QobuzTrack {
            info: TrackInfo {
                identifier,
                is_seekable: true,
                author,
                length: length as u64,
                is_stream: false,
                position: 0,
                title,
                uri: Some(uri),
                artwork_url,
                isrc,
                source_name: "qobuz".to_string(),
            },
            album_name,
            album_url,
            artist_url,
            artist_artwork_url,
            token_tracker: self.token_tracker.clone(),
            client: self.client.clone(),
        }
    }

    async fn handle_search(&self, query: &str) -> LoadResult {
        match self
            .api_request(
                "catalog/search",
                vec![
                    ("query", query.to_string()),
                    ("limit", self.search_limit.to_string()),
                    ("type", "tracks".to_string()),
                ],
            )
            .await
        {
            Ok(json) => {
                let items = json["tracks"]["items"].as_array();
                if items.map(|a| a.is_empty()).unwrap_or(true) {
                    return LoadResult::Empty {};
                }
                let tracks: Vec<Track> = items
                    .unwrap()
                    .iter()
                    .map(|item| Track::new(self.parse_qobuz_track(item).info))
                    .collect();
                LoadResult::Search(tracks)
            }
            Err(e) => {
                error!("Qobuz search error: {}", e);
                LoadResult::Empty {}
            }
        }
    }

    async fn handle_isrc(&self, isrc: &str) -> LoadResult {
        match self
            .api_request(
                "catalog/search",
                vec![
                    ("query", isrc.to_string()),
                    ("limit", "15".to_string()),
                    ("type", "tracks".to_string()),
                ],
            )
            .await
        {
            Ok(json) => {
                let items = json["tracks"]["items"].as_array();
                if items.map(|a| a.is_empty()).unwrap_or(true) {
                    return LoadResult::Empty {};
                }
                let track = Track::new(self.parse_qobuz_track(&items.unwrap()[0]).info);
                LoadResult::Track(track)
            }
            Err(e) => {
                error!("Qobuz ISRC search error: {}", e);
                LoadResult::Empty {}
            }
        }
    }

    async fn handle_recommendations(&self, track_id: &str) -> LoadResult {
        let track_json = match self
            .api_request("track/get", vec![("track_id", track_id.to_string())])
            .await
        {
            Ok(j) => j,
            Err(_) => return LoadResult::Empty {},
        };

        let artist_id = track_json["performer"]["id"]
            .as_i64()
            .or_else(|| track_json["artist"]["id"].as_i64())
            .unwrap_or(0);

        let track_id_i64 = track_id.parse::<i64>().unwrap_or(0);

        let payload = json!({
            "limit": 50,
            "listened_tracks_ids": [track_id_i64],
            "track_to_analysed": [
                {
                    "track_id": track_id_i64,
                    "artist_id": artist_id
                }
            ]
        });

        let tokens = match self.token_tracker.get_tokens().await {
            Some(t) => t,
            None => return LoadResult::Empty {},
        };

        let mut request = self
            .base_request(self.client.post(format!("{}dynamic/suggest", API_URL)))
            .header("Accept", "application/json")
            .header("x-app-id", &tokens.app_id)
            .json(&payload);

        if let Some(user_token) = &tokens.user_token {
            request = request.header("x-user-auth-token", user_token);
        }

        let resp = match request.send().await {
            Ok(r) => r,
            Err(e) => {
                error!("Qobuz recommendations request error: {}", e);
                return LoadResult::Empty {};
            }
        };

        if !resp.status().is_success() {
            return LoadResult::Empty {};
        }

        let json: Value = resp.json().await.unwrap_or(json!({}));
        let items = json["tracks"]["items"].as_array();
        if items.map(|a| a.is_empty()).unwrap_or(true) {
            return LoadResult::Empty {};
        }

        let tracks: Vec<Track> = items
            .unwrap()
            .iter()
            .map(|item| Track::new(self.parse_qobuz_track(item).info))
            .collect();

        LoadResult::Playlist(PlaylistData {
            info: PlaylistInfo {
                name: "Qobuz Recommendations".to_string(),
                selected_track: -1,
            },
            plugin_info: json!({
                "type": "recommendations",
                "totalTracks": tracks.len()
            }),
            tracks,
        })
    }

    async fn handle_album(&self, id: &str) -> LoadResult {
        match self
            .api_request(
                "album/get",
                vec![
                    ("album_id", id.to_string()),
                    ("limit", self.album_load_limit.to_string()),
                    ("offset", "0".to_string()),
                ],
            )
            .await
        {
            Ok(mut json) => {
                let title = json["title"]
                    .as_str()
                    .unwrap_or("Unknown Album")
                    .to_string();
                let author = json["artist"]["name"]
                    .as_str()
                    .or_else(|| json["artist"]["name"]["display"].as_str())
                    .unwrap_or("Unknown Artist")
                    .to_string();
                let artwork_url = json["image"]["large"].as_str().map(|s| s.to_string());
                let uri = format!("https://open.qobuz.com/album/{}", id);

                let tracks_json = json["tracks"]["items"].take();
                if tracks_json.as_array().map(|a| a.is_empty()).unwrap_or(true) {
                    return LoadResult::Empty {};
                }

                let tracks: Vec<Track> = tracks_json
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|item| {
                        let mut item = item.clone();
                        item["album"] = json.clone();
                        Track::new(self.parse_qobuz_track(&item).info)
                    })
                    .collect();

                let track_count = tracks.len();
                LoadResult::Playlist(PlaylistData {
                    info: PlaylistInfo {
                        name: title,
                        selected_track: -1,
                    },
                    plugin_info: json!({
                        "type": "album",
                        "url": uri,
                        "artworkUrl": artwork_url,
                        "author": author,
                        "totalTracks": track_count
                    }),
                    tracks,
                })
            }
            Err(_) => LoadResult::Empty {},
        }
    }

    async fn handle_playlist(&self, id: &str) -> LoadResult {
        match self
            .api_request(
                "playlist/get",
                vec![
                    ("playlist_id", id.to_string()),
                    ("extra", "tracks".to_string()),
                    ("limit", self.playlist_load_limit.to_string()),
                    ("offset", "0".to_string()),
                ],
            )
            .await
        {
            Ok(json) => {
                let items = json["tracks"]["items"].as_array();
                if items.map(|a| a.is_empty()).unwrap_or(true) {
                    return LoadResult::Empty {};
                }

                let name = json["name"]
                    .as_str()
                    .unwrap_or("Unknown Playlist")
                    .to_string();
                let author = json["owner"]["name"]
                    .as_str()
                    .unwrap_or("Unknown")
                    .to_string();
                let artwork_url = json["images300"]
                    .as_array()
                    .and_then(|a| a.first())
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let url = json["url"]
                    .as_str()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| format!("https://open.qobuz.com/playlist/{}", id));

                let tracks: Vec<Track> = items
                    .unwrap()
                    .iter()
                    .map(|item| Track::new(self.parse_qobuz_track(item).info))
                    .collect();

                let track_count = tracks.len();
                LoadResult::Playlist(PlaylistData {
                    info: PlaylistInfo {
                        name,
                        selected_track: -1,
                    },
                    plugin_info: json!({
                        "type": "playlist",
                        "url": url,
                        "artworkUrl": artwork_url,
                        "author": author,
                        "totalTracks": track_count
                    }),
                    tracks,
                })
            }
            Err(_) => LoadResult::Empty {},
        }
    }

    async fn handle_artist(&self, id: &str) -> LoadResult {
        match self
            .api_request("artist/page", vec![("artist_id", id.to_string())])
            .await
        {
            Ok(json) => {
                let top_tracks = json["top_tracks"].as_array();
                if top_tracks.map(|a| a.is_empty()).unwrap_or(true) {
                    return LoadResult::Empty {};
                }

                let name = json["name"]["display"]
                    .as_str()
                    .or_else(|| json["name"].as_str())
                    .unwrap_or("Unknown Artist")
                    .to_string();

                let artwork_url = json["images"]["potrait"]["hash"]
                    .as_str()
                    .filter(|h| !h.is_empty())
                    .map(|h| {
                        format!(
                            "https://static.qobuz.com/images/artists/covers/large/{}.jpg",
                            h
                        )
                    });

                let uri = format!("https://open.qobuz.com/artist/{}", id);

                let tracks: Vec<Track> = top_tracks
                    .unwrap()
                    .iter()
                    .take(self.artist_load_limit)
                    .map(|item| Track::new(self.parse_qobuz_track(item).info))
                    .collect();

                let track_count = tracks.len();
                LoadResult::Playlist(PlaylistData {
                    info: PlaylistInfo {
                        name: format!("{}'s Top Tracks", name),
                        selected_track: -1,
                    },
                    plugin_info: json!({
                        "type": "artist",
                        "url": uri,
                        "artworkUrl": artwork_url,
                        "author": name,
                        "totalTracks": track_count
                    }),
                    tracks,
                })
            }
            Err(_) => LoadResult::Empty {},
        }
    }
}

#[async_trait]
impl SourcePlugin for QobuzSource {
    fn name(&self) -> &str {
        "qobuz"
    }

    fn can_handle(&self, identifier: &str) -> bool {
        self.search_prefixes
            .iter()
            .any(|p| identifier.starts_with(p))
            || self.isrc_prefixes.iter().any(|p| identifier.starts_with(p))
            || self.rec_prefixes.iter().any(|p| identifier.starts_with(p))
            || self.url_regex.is_match(identifier)
    }

    fn search_prefixes(&self) -> Vec<&str> {
        self.search_prefixes.iter().map(|s| s.as_str()).collect()
    }

    fn isrc_prefixes(&self) -> Vec<&str> {
        self.isrc_prefixes.iter().map(|s| s.as_str()).collect()
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
            return self.handle_search(&identifier[prefix.len()..]).await;
        }
        if let Some(prefix) = self
            .isrc_prefixes
            .iter()
            .find(|p| identifier.starts_with(*p))
        {
            return self.handle_isrc(&identifier[prefix.len()..]).await;
        }
        if let Some(prefix) = self
            .rec_prefixes
            .iter()
            .find(|p| identifier.starts_with(*p))
        {
            return self
                .handle_recommendations(&identifier[prefix.len()..])
                .await;
        }

        if let Some(caps) = self.url_regex.captures(identifier) {
            let type_ = caps
                .name("type")
                .or_else(|| caps.name("type2"))
                .map(|m| m.as_str())
                .unwrap_or("");
            let id = caps
                .name("id")
                .or_else(|| caps.name("id2"))
                .map(|m| m.as_str())
                .unwrap_or("");

            return match type_ {
                "track" => {
                    match self
                        .api_request("track/get", vec![("track_id", id.to_string())])
                        .await
                    {
                        Ok(json) => {
                            LoadResult::Track(Track::new(self.parse_qobuz_track(&json).info))
                        }
                        Err(_) => LoadResult::Empty {},
                    }
                }
                "album" => self.handle_album(id).await,
                "playlist" => self.handle_playlist(id).await,
                "artist" => self.handle_artist(id).await,
                _ => LoadResult::Empty {},
            };
        }

        LoadResult::Empty {}
    }

    async fn get_track(
        &self,
        identifier: &str,
        _routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<BoxedTrack> {
        let id = if identifier.contains("qobuz.com/track/") {
            identifier
                .split("track/")
                .nth(1)?
                .split('/')
                .next()?
                .split('?')
                .next()?
        } else {
            identifier
        };

        let tokens = self.token_tracker.get_tokens().await?;
        if tokens.user_token.is_none() {
            debug!("Qobuz: No user token, returning None to trigger mirroring");
            return None;
        }

        match self
            .api_request("track/get", vec![("track_id", id.to_string())])
            .await
        {
            Ok(json) => Some(Box::new(self.parse_qobuz_track(&json))),
            Err(_) => None,
        }
    }
}
