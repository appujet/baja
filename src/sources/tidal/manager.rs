use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use regex::Regex;
use serde_json::Value;

use super::{client::TidalClient, oauth::TidalOAuth, token::TidalTokenTracker, track::TidalTrack};
use crate::{
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
                .rec_prefixes()
                .iter()
                .any(|p| identifier.starts_with(p))
            || url_regex().is_match(identifier)
    }

    fn search_prefixes(&self) -> Vec<&str> {
        vec!["tdsearch:"]
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
        let (id, _type) = if let Some(caps) = url_regex().captures(identifier) {
            let type_str = caps.get(1).map_or("", |m| m.as_str());
            let id = caps.get(2).map_or("", |m| m.as_str());
            if type_str != "track" {
                return None;
            }
            (id.to_owned(), type_str.to_owned())
        } else {
            (identifier.to_owned(), "track".to_owned())
        };

        Some(Box::new(TidalTrack {
            identifier: id,
            client: self.client.clone(),
        }))
    }
}
