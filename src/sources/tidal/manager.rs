use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose};
use regex::Regex;
use serde_json::Value;
use tracing::{debug, warn};

use super::{
    client::TidalClient,
    model::{Manifest, PlaybackInfo},
    oauth::TidalOAuth,
    token::TidalTokenTracker,
    track::TidalTrack,
};
use crate::{
    common::types::AudioFormat,
    protocol::tracks::{LoadResult, PlaylistData, PlaylistInfo, Track, TrackInfo},
    sources::SourcePlugin,
};

fn url_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"https?://(?:(?:listen|www)\.)?tidal\.com/(?:browse/)?(album|track|playlist|mix|artist)/([a-zA-Z0-9\-]+)(?:/.*)?(?:\?.*)?").unwrap()
    })
}

pub struct TidalSource {
    pub client: Arc<TidalClient>,
    playlist_load_limit: usize,
    album_load_limit: usize,
    artist_load_limit: usize,
}

impl TidalSource {
    pub fn new(
        config: Option<crate::config::TidalConfig>,
        http_client: Arc<reqwest::Client>,
    ) -> Result<Self, String> {
        let (country, quality, p_limit, a_limit, art_limit, refresh_token, get_oauth_token) =
            if let Some(c) = config {
                (
                    c.country_code,
                    c.quality,
                    c.playlist_load_limit,
                    c.album_load_limit,
                    c.artist_load_limit,
                    c.refresh_token,
                    c.get_oauth_token,
                )
            } else {
                (
                    "US".to_string(),
                    crate::config::sources::default_tidal_quality(),
                    0,
                    0,
                    0,
                    None,
                    false,
                )
            };

        let oauth = Arc::new(TidalOAuth::new(refresh_token));

        if get_oauth_token {
            let oauth_clone = oauth.clone();
            tokio::spawn(async move {
                oauth_clone.initialize_access_token().await;
            });
        }

        let token_tracker = Arc::new(TidalTokenTracker::new(http_client.clone(), oauth));
        token_tracker.clone().init();

        let client = Arc::new(TidalClient::new(
            http_client,
            token_tracker,
            country,
            quality,
        ));

        Ok(Self {
            client,
            playlist_load_limit: p_limit,
            album_load_limit: a_limit,
            artist_load_limit: art_limit,
        })
    }

    fn parse_track(&self, item: &Value) -> Option<TrackInfo> {
        let id = item.get("id")?.as_u64()?.to_string();
        let title = item
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown Title")
            .to_string();

        let artists = item
            .get("artists")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.get("name").and_then(|n| n.as_str()))
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_else(|| "Unknown Artist".to_owned());

        let length = item.get("duration").and_then(|v| v.as_u64()).unwrap_or(0) * 1000;
        let isrc = item
            .get("isrc")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_owned());

        let artwork_url = item
            .get("album")
            .and_then(|a| a.get("cover"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| {
                format!(
                    "https://resources.tidal.com/images/{}/1280x1280.jpg",
                    s.replace("-", "/")
                )
            });

        let url = item
            .get("url")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.replace("http://", "https://"));

        Some(TrackInfo {
            title,
            author: artists,
            length,
            identifier: id,
            is_stream: false,
            uri: url,
            artwork_url,
            isrc,
            source_name: "tidal".to_owned(),
            is_seekable: true,
            position: 0,
        })
    }

    async fn get_track_data(&self, id: &str) -> LoadResult {
        match self.client.get_json(&format!("/tracks/{id}")).await {
            Ok(data) => self
                .parse_track(&data)
                .map(|i| LoadResult::Track(Track::new(i)))
                .unwrap_or(LoadResult::Empty {}),
            Err(_) => LoadResult::Empty {},
        }
    }

    async fn get_album_or_playlist(&self, id: &str, type_str: &str) -> LoadResult {
        let info_data = match self.client.get_json(&format!("/{type_str}s/{id}")).await {
            Ok(d) => d,
            Err(_) => return LoadResult::Empty {},
        };

        let title = info_data
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown")
            .to_owned();
        let limit = (if type_str == "album" {
            self.album_load_limit
        } else {
            self.playlist_load_limit
        })
        .clamp(1, 100);

        let tracks_data = match self
            .client
            .get_json(&format!("/{type_str}s/{id}/tracks?limit={limit}"))
            .await
        {
            Ok(d) => d,
            Err(_) => return LoadResult::Empty {},
        };

        let mut tracks = Vec::new();
        if let Some(items) = tracks_data.get("items").and_then(|v| v.as_array()) {
            for item in items {
                let track_obj = item.get("item").unwrap_or(item);
                if let Some(info) = self.parse_track(track_obj) {
                    tracks.push(Track::new(info));
                }
            }
        }

        if tracks.is_empty() {
            return LoadResult::Empty {};
        }

        LoadResult::Playlist(PlaylistData {
            info: PlaylistInfo {
                name: title,
                selected_track: -1,
            },
            plugin_info: serde_json::json!({
                "type": type_str,
                "url": format!("https://tidal.com/browse/{type_str}/{id}"),
                "totalTracks": info_data.get("numberOfTracks").or_else(|| info_data.get("numberOfSongs")).and_then(|v| v.as_u64()).unwrap_or(tracks.len() as u64)
            }),
            tracks,
        })
    }

    async fn get_mix(&self, id: &str, name_override: Option<String>) -> LoadResult {
        let data = match self
            .client
            .get_json(&format!("/mixes/{id}/items?limit=100"))
            .await
        {
            Ok(d) => d,
            Err(_) => return LoadResult::Empty {},
        };

        let mut tracks = Vec::new();
        if let Some(items) = data.get("items").and_then(|v| v.as_array()) {
            for item in items {
                let track_obj = item.get("item").unwrap_or(item);
                if let Some(info) = self.parse_track(track_obj) {
                    tracks.push(Track::new(info));
                }
            }
        }

        if tracks.is_empty() {
            return LoadResult::Empty {};
        }

        LoadResult::Playlist(PlaylistData {
            info: PlaylistInfo {
                name: name_override.unwrap_or_else(|| format!("Mix: {id}")),
                selected_track: -1,
            },
            plugin_info: serde_json::json!({ "type": "playlist", "url": format!("https://tidal.com/browse/mix/{id}"), "totalTracks": tracks.len() }),
            tracks,
        })
    }

    async fn resolve_by_isrc(&self, isrc: &str) -> LoadResult {
        // v2 only; requires OAuth Bearer. scraper token won't work here.
        let token = match self.client.token_tracker.get_oauth_token().await {
            Some(t) => t,
            None => {
                warn!("Tidal ISRC lookup requires an OAuth token; none configured");
                return LoadResult::Empty {};
            }
        };

        let url = format!(
            "https://openapi.tidal.com/v2/tracks?countryCode={}",
            self.client.country_code
        );

        let resp = self
            .client
            .base_request(self.client.inner.get(&url))
            .query(&[("filter[isrc]", isrc)])
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await;

        let r = match resp {
            Ok(r) => r,
            Err(e) => {
                debug!("Tidal ISRC request failed: {e}");
                return LoadResult::Empty {};
            }
        };

        if !r.status().is_success() {
            debug!(
                "Tidal ISRC response {}: {}",
                r.status(),
                r.text().await.unwrap_or_default()
            );
            return LoadResult::Empty {};
        }

        let data: serde_json::Value = match r.json().await {
            Ok(d) => d,
            Err(e) => {
                debug!("Tidal ISRC response parse error: {e}");
                return LoadResult::Empty {};
            }
        };

        match data.pointer("/data/0/id").and_then(|v| v.as_str()) {
            Some(id) => self.get_track_data(id).await,
            None => {
                debug!("Tidal ISRC={isrc} not found in catalog");
                LoadResult::Empty {}
            }
        }
    }

    async fn search(&self, query: &str) -> LoadResult {
        let encoded = urlencoding::encode(query);
        match self
            .client
            .get_json(&format!("/search?query={encoded}&limit=10&types=TRACKS"))
            .await
        {
            Ok(data) => {
                let mut tracks = Vec::new();
                if let Some(items) = data.pointer("/tracks/items").and_then(|v| v.as_array()) {
                    for item in items {
                        if let Some(info) = self.parse_track(item) {
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
            Err(_) => LoadResult::Empty {},
        }
    }

    async fn get_recommendations(&self, id: &str) -> LoadResult {
        if let Ok(data) = self.client.get_json(&format!("/tracks/{id}")).await
            && let Some(mix_id) = data.pointer("/mixes/TRACK_MIX").and_then(|v| v.as_str())
        {
            return self
                .get_mix(mix_id, Some("Tidal Recommendations".to_string()))
                .await;
        }
        LoadResult::Empty {}
    }

    async fn get_artist_top_tracks(&self, id: &str) -> LoadResult {
        let info_data = match self.client.get_json(&format!("/artists/{id}")).await {
            Ok(d) => d,
            Err(_) => return LoadResult::Empty {},
        };

        let name = info_data
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown Artist");
        let limit = self.artist_load_limit.clamp(1, 10);

        let data = match self
            .client
            .get_json(&format!("/artists/{id}/toptracks?limit={limit}"))
            .await
        {
            Ok(d) => d,
            Err(_) => return LoadResult::Empty {},
        };

        let mut tracks = Vec::new();
        if let Some(items) = data.get("items").and_then(|v| v.as_array()) {
            for item in items {
                if let Some(info) = self.parse_track(item) {
                    tracks.push(Track::new(info));
                }
            }
        }

        if tracks.is_empty() {
            return LoadResult::Empty {};
        }

        LoadResult::Playlist(PlaylistData {
            info: PlaylistInfo {
                name: format!("{name}'s Top Tracks"),
                selected_track: -1,
            },
            plugin_info: serde_json::json!({ "type": "artist", "url": format!("https://tidal.com/browse/artist/{id}"), "totalTracks": tracks.len() }),
            tracks,
        })
    }
}

#[async_trait]
impl SourcePlugin for TidalSource {
    fn name(&self) -> &str {
        "tidal"
    }

    fn can_handle(&self, identifier: &str) -> bool {
        self.search_prefixes()
            .iter()
            .any(|p| identifier.starts_with(p))
            || self
                .isrc_prefixes()
                .iter()
                .any(|p| identifier.starts_with(p))
            || self
                .rec_prefixes()
                .iter()
                .any(|p| identifier.starts_with(p))
            || url_regex().is_match(identifier)
    }

    fn search_prefixes(&self) -> Vec<&str> {
        vec!["tdsearch:"]
    }
    fn isrc_prefixes(&self) -> Vec<&str> {
        vec!["tdisrc:"]
    }
    fn rec_prefixes(&self) -> Vec<&str> {
        vec!["tdrec:"]
    }
    fn is_mirror(&self) -> bool {
        false
    }

    async fn load(
        &self,
        identifier: &str,
        _: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> LoadResult {
        if let Some(prefix) = self
            .search_prefixes()
            .iter()
            .find(|p| identifier.starts_with(**p))
        {
            return self.search(&identifier[prefix.len()..]).await;
        }

        if let Some(prefix) = self
            .isrc_prefixes()
            .iter()
            .find(|p| identifier.starts_with(**p))
        {
            return self.resolve_by_isrc(&identifier[prefix.len()..]).await;
        }

        if let Some(prefix) = self
            .rec_prefixes()
            .iter()
            .find(|p| identifier.starts_with(**p))
        {
            return self.get_recommendations(&identifier[prefix.len()..]).await;
        }

        if let Some(caps) = url_regex().captures(identifier) {
            let type_str = caps.get(1).map_or("", |m| m.as_str());
            let id = caps.get(2).map_or("", |m| m.as_str());

            return match type_str {
                "track" => self.get_track_data(id).await,
                "album" | "playlist" => self.get_album_or_playlist(id, type_str).await,
                "mix" => self.get_mix(id, None).await,
                "artist" => self.get_artist_top_tracks(id).await,
                _ => LoadResult::Empty {},
            };
        }

        LoadResult::Empty {}
    }

    async fn get_track(
        &self,
        identifier: &str,
        _: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<crate::sources::plugin::BoxedTrack> {
        let id = if let Some(caps) = url_regex().captures(identifier) {
            let type_str = caps.get(1).map_or("", |m| m.as_str());
            let id = caps.get(2).map_or("", |m| m.as_str());
            if type_str != "track" {
                return None;
            }
            id.to_owned()
        } else {
            identifier.to_owned()
        };

        let token = match self.client.token_tracker.get_oauth_token().await {
            Some(t) => t,
            None => {
                warn!("Tidal playback requires an OAuth login");
                return None;
            }
        };

        let quality = &self.client.quality;
        let url = format!(
            "https://api.tidal.com/v1/tracks/{}/playbackinfo?audioquality={}&playbackmode=STREAM&assetpresentation=FULL&countryCode={}",
            id, quality, self.client.country_code
        );

        debug!("Tidal: Resolving playback info for {}", id);

        let resp = match self
            .client
            .inner
            .get(&url)
            .header("Authorization", format!("Bearer {token}"))
            .header("User-Agent", "TIDAL/3704 CFNetwork/1220.1 Darwin/20.3.0")
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                warn!("Tidal: Failed to fetch playback info: {}", e);
                return None;
            }
        };

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            warn!("Tidal: Playback API returned {}: {}", status, body);
            return None;
        }

        let info: PlaybackInfo = match resp.json().await {
            Ok(i) => i,
            Err(e) => {
                warn!("Tidal: Failed to parse playback info: {}", e);
                return None;
            }
        };

        let decoded = match general_purpose::STANDARD.decode(&info.manifest) {
            Ok(d) => d,
            Err(e) => {
                warn!("Tidal: Failed to decode manifest: {}", e);
                return None;
            }
        };

        let manifest: Manifest = match serde_json::from_slice(&decoded) {
            Ok(m) => m,
            Err(e) => {
                warn!("Tidal: Failed to parse manifest JSON: {}", e);
                return None;
            }
        };

        let stream_url = match manifest.urls.first() {
            Some(u) => u.clone(),
            None => {
                warn!("Tidal: No stream URL in manifest");
                return None;
            }
        };

        let mut kind = AudioFormat::from_url(&stream_url);
        if kind == AudioFormat::Unknown {
            if quality == "HI_RES_LOSSLESS" {
                kind = AudioFormat::Flac;
            } else if quality == "LOSSLESS" {
                kind = AudioFormat::Mp4;
            } else {
                kind = AudioFormat::Aac;
            }
        }

        Some(Box::new(TidalTrack {
            identifier: id,
            stream_url,
            kind,
            client: self.client.clone(),
        }))
    }
}
