use std::sync::Arc;

use async_trait::async_trait;
use regex::Regex;
use serde_json::json;
use tracing::debug;

use super::{
    api::AmazonMusicClient,
    parsers::{
        parse_album_tracks, parse_artist_top_songs, parse_community_playlist_tracks,
        parse_playlist_tracks, parse_search_tracks, parse_track,
    },
    validators::{
        is_invalid_album, is_invalid_artist, is_invalid_community_playlist, is_invalid_playlist,
        is_invalid_track,
    },
};
use crate::{
    config::AmazonMusicConfig,
    protocol::tracks::{LoadError, LoadResult, PlaylistData, PlaylistInfo, Track},
    sources::{SourcePlugin, plugin::BoxedTrack},
};

const TRACK_RE: &str = r"(?i)^https?://(?:www\.)?music\.amazon\.[a-z.]+/tracks/([A-Z0-9]{10,20})";
const ALBUM_RE: &str = r"(?i)^https?://(?:www\.)?music\.amazon\.[a-z.]+/albums/([A-Z0-9]{10,20})";
const ARTIST_RE: &str = r"(?i)^https?://(?:www\.)?music\.amazon\.[a-z.]+/artists/([A-Z0-9]{10,20})";
const PLAYLIST_RE: &str =
    r"(?i)^https?://(?:www\.)?music\.amazon\.[a-z.]+/playlists/([A-Z0-9]{10,20})";
const USER_PLAYLIST_RE: &str =
    r"(?i)^https?://(?:www\.)?music\.amazon\.[a-z.]+/user-playlists/([a-zA-Z0-9]+)";
const DOMAIN_RE: &str = r"(?i)^https?://(?:www\.)?music\.amazon\.";

pub struct AmazonMusicSource {
    client: Arc<AmazonMusicClient>,
    search_limit: usize,
    proxy: Option<crate::config::HttpProxyConfig>,
    track_re: Regex,
    album_re: Regex,
    artist_re: Regex,
    playlist_re: Regex,
    user_playlist_re: Regex,
    domain_re: Regex,
}

impl AmazonMusicSource {
    pub fn new(config: AmazonMusicConfig, http: Arc<reqwest::Client>) -> Result<Self, String> {
        Ok(Self {
            client: Arc::new(AmazonMusicClient::new(http)),
            search_limit: config.search_limit.min(5),
            proxy: config.proxy,
            track_re: Regex::new(TRACK_RE).map_err(|e| e.to_string())?,
            album_re: Regex::new(ALBUM_RE).map_err(|e| e.to_string())?,
            artist_re: Regex::new(ARTIST_RE).map_err(|e| e.to_string())?,
            playlist_re: Regex::new(PLAYLIST_RE).map_err(|e| e.to_string())?,
            user_playlist_re: Regex::new(USER_PLAYLIST_RE).map_err(|e| e.to_string())?,
            domain_re: Regex::new(DOMAIN_RE).map_err(|e| e.to_string())?,
        })
    }

    fn capture_id(&self, re: &Regex, url: &str) -> Option<String> {
        re.captures(url)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().to_string())
    }

    async fn load_track(&self, url: &str) -> LoadResult {
        let id = match self.capture_id(&self.track_re, url) {
            Some(id) => id,
            None => return LoadResult::Empty {},
        };

        let resp = match self.client.fetch_track(&id).await {
            Some(v) => v,
            None => {
                return LoadResult::Error(LoadError {
                    message: Some(format!("Amazon Music: failed to fetch track '{id}'")),
                    severity: crate::common::Severity::Suspicious,
                    cause: "API request failed".to_string(),
                    cause_stack_trace: None,
                });
            }
        };

        if is_invalid_track(&resp) {
            debug!("Amazon Music: track '{id}' not found or no longer available");
            return LoadResult::Empty {};
        }

        match parse_track(&resp, &id) {
            Some(info) => LoadResult::Track(Track::new(info)),
            None => {
                debug!("Amazon Music: failed to parse track '{id}'");
                LoadResult::Empty {}
            }
        }
    }

    async fn load_album(&self, url: &str) -> LoadResult {
        let album_id = match self.capture_id(&self.album_re, url) {
            Some(id) => id,
            None => return LoadResult::Empty {},
        };

        let resp = match self.client.fetch_album(&album_id).await {
            Some(v) => v,
            None => return LoadResult::Empty {},
        };

        if is_invalid_album(&resp) {
            debug!("Amazon Music: album '{album_id}' not found");
            return LoadResult::Empty {};
        }

        let (album_name, artist_name, track_infos) = match parse_album_tracks(&resp, &album_id) {
            Some(r) => r,
            None => return LoadResult::Empty {},
        };

        let artwork = resp["methods"][0]["template"]["headerImage"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(super::api::clean_image_url);

        let tracks: Vec<Track> = track_infos.into_iter().map(Track::new).collect();
        if tracks.is_empty() {
            return LoadResult::Empty {};
        }

        LoadResult::Playlist(PlaylistData {
            info: PlaylistInfo {
                name: album_name.clone(),
                selected_track: -1,
            },
            plugin_info: json!({
                "type": "album",
                "url": format!("https://music.amazon.com/albums/{album_id}"),
                "artworkUrl": artwork,
                "author": artist_name,
                "totalTracks": tracks.len()
            }),
            tracks,
        })
    }

    async fn load_artist(&self, url: &str) -> LoadResult {
        let artist_id = match self.capture_id(&self.artist_re, url) {
            Some(id) => id,
            None => return LoadResult::Empty {},
        };

        let resp = match self.client.fetch_artist(&artist_id).await {
            Some(v) => v,
            None => return LoadResult::Empty {},
        };

        if is_invalid_artist(&resp) {
            debug!("Amazon Music: artist '{artist_id}' not found");
            return LoadResult::Empty {};
        }

        let unique_album_ids: Vec<String> = {
            let widgets = match resp["methods"][0]["template"]["widgets"].as_array() {
                Some(w) => w,
                None => return LoadResult::Empty {},
            };
            let top_songs = match widgets.iter().find(|w| {
                w["header"]
                    .as_str()
                    .map(|h| h.to_lowercase().contains("top songs"))
                    .unwrap_or(false)
            }) {
                Some(w) => w,
                None => return LoadResult::Empty {},
            };

            let mut seen = std::collections::HashSet::new();
            top_songs["items"]
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| {
                            let key = item["iconButton"]["observer"]["storageKey"].as_str()?;
                            let album_id = key.split(':').next()?.to_string();
                            if !album_id.is_empty() && seen.insert(album_id.clone()) {
                                Some(album_id)
                            } else {
                                None
                            }
                        })
                        .collect()
                })
                .unwrap_or_default()
        };

        let fetch_futures: Vec<_> = unique_album_ids
            .iter()
            .map(|album_id| self.client.fetch_album(album_id))
            .collect();

        let album_responses = futures::future::join_all(fetch_futures).await;

        let mut duration_map: std::collections::HashMap<String, u64> =
            std::collections::HashMap::new();

        for (album_id, album_resp) in unique_album_ids.iter().zip(album_responses) {
            let album_resp = match album_resp {
                Some(v) => v,
                None => continue,
            };
            let album_items =
                match album_resp["methods"][0]["template"]["widgets"][0]["items"].as_array() {
                    Some(i) => i.clone(),
                    None => continue,
                };
            for track in &album_items {
                let deeplink = match track["primaryTextLink"]["deeplink"].as_str() {
                    Some(dl) => dl,
                    None => continue,
                };
                let track_id = match deeplink.split("/tracks/").nth(1) {
                    Some(id) => id.split('/').next().unwrap_or("").to_string(),
                    None => continue,
                };
                if track_id.is_empty() {
                    continue;
                }
                let duration_ms =
                    super::api::duration_str_to_ms(track["secondaryText3"].as_str().unwrap_or(""));
                duration_map.insert(format!("{album_id}:{track_id}"), duration_ms);
            }
        }

        let result = match parse_artist_top_songs(&resp, &artist_id, &duration_map) {
            Some(r) => r,
            None => return LoadResult::Empty {},
        };

        let tracks: Vec<Track> = result.tracks.into_iter().map(Track::new).collect();
        if tracks.is_empty() {
            return LoadResult::Empty {};
        }

        LoadResult::Playlist(PlaylistData {
            info: PlaylistInfo {
                name: format!("{}'s Top Songs", result.name),
                selected_track: -1,
            },
            plugin_info: json!({
                "type": "artist",
                "url": format!("https://music.amazon.com/artists/{artist_id}"),
                "artworkUrl": result.artwork_url,
                "author": result.name,
                "totalTracks": tracks.len()
            }),
            tracks,
        })
    }

    async fn load_playlist(&self, url: &str) -> LoadResult {
        let playlist_id = match self.capture_id(&self.playlist_re, url) {
            Some(id) => id,
            None => return LoadResult::Empty {},
        };

        let resp = match self.client.fetch_playlist(&playlist_id).await {
            Some(v) => v,
            None => return LoadResult::Empty {},
        };

        if is_invalid_playlist(&resp) {
            debug!("Amazon Music: playlist '{playlist_id}' not found/unavailable");
            return LoadResult::Empty {};
        }

        let result = match parse_playlist_tracks(&resp) {
            Some(r) => r,
            None => return LoadResult::Empty {},
        };

        let tracks: Vec<Track> = result.tracks.into_iter().map(Track::new).collect();
        if tracks.is_empty() {
            return LoadResult::Empty {};
        }

        LoadResult::Playlist(PlaylistData {
            info: PlaylistInfo {
                name: result.name.clone(),
                selected_track: -1,
            },
            plugin_info: json!({
                "type": "playlist",
                "url": format!("https://music.amazon.com/playlists/{playlist_id}"),
                "artworkUrl": result.artwork_url,
                "author": "Amazon Music",
                "totalTracks": tracks.len()
            }),
            tracks,
        })
    }

    async fn load_community_playlist(&self, url: &str) -> LoadResult {
        let playlist_id = match self.capture_id(&self.user_playlist_re, url) {
            Some(id) => id,
            None => return LoadResult::Empty {},
        };

        let resp = match self.client.fetch_community_playlist(&playlist_id).await {
            Some(v) => v,
            None => return LoadResult::Empty {},
        };

        if is_invalid_community_playlist(&resp) {
            debug!("Amazon Music: community playlist '{playlist_id}' not found");
            return LoadResult::Empty {};
        }

        let result = match parse_community_playlist_tracks(&resp) {
            Some(r) => r,
            None => return LoadResult::Empty {},
        };

        let tracks: Vec<Track> = result.tracks.into_iter().map(Track::new).collect();
        if tracks.is_empty() {
            return LoadResult::Empty {};
        }

        LoadResult::Playlist(PlaylistData {
            info: PlaylistInfo {
                name: result.name.clone(),
                selected_track: -1,
            },
            plugin_info: json!({
                "type": "playlist",
                "url": format!("https://music.amazon.com/user-playlists/{playlist_id}"),
                "artworkUrl": result.artwork_url,
                "author": "Community User",
                "totalTracks": tracks.len()
            }),
            tracks,
        })
    }

    async fn load_search(&self, query: &str) -> LoadResult {
        let resp = match self.client.search_tracks(query).await {
            Some(v) => v,
            None => return LoadResult::Empty {},
        };

        let items: Vec<serde_json::Value> = match resp["methods"]
            .as_array()
            .and_then(|m| m.first())
            .and_then(|m| m["template"]["widgets"].as_array())
            .and_then(|w| w.first())
            .and_then(|w| w["items"].as_array())
        {
            Some(i) => i.iter().take(self.search_limit).cloned().collect(),
            None => return LoadResult::Empty {},
        };

        let mut unique_albums: std::collections::HashSet<String> = std::collections::HashSet::new();
        for item in &items {
            if let Some(key) = item["iconButton"]["observer"]["storageKey"].as_str()
                && let Some(album_id) = key.split(':').next()
                && !album_id.is_empty()
            {
                unique_albums.insert(album_id.to_string());
            }
        }

        let album_ids: Vec<String> = unique_albums.into_iter().collect();
        let mut duration_map: std::collections::HashMap<String, u64> =
            std::collections::HashMap::new();

        for batch in album_ids.chunks(5) {
            let futures: Vec<_> = batch
                .iter()
                .map(|album_id| self.client.fetch_album(album_id))
                .collect();

            let results = futures::future::join_all(futures).await;

            for (album_id, album_resp) in batch.iter().zip(results) {
                let album_resp = match album_resp {
                    Some(v) => v,
                    None => continue,
                };

                let album_items =
                    match album_resp["methods"][0]["template"]["widgets"][0]["items"].as_array() {
                        Some(i) => i.clone(),
                        None => continue,
                    };

                for track in &album_items {
                    let deeplink = match track["primaryTextLink"]["deeplink"].as_str() {
                        Some(dl) => dl,
                        None => continue,
                    };
                    let track_id = match deeplink.split("/tracks/").nth(1) {
                        Some(id) => id.split('/').next().unwrap_or("").to_string(),
                        None => continue,
                    };
                    if track_id.is_empty() {
                        continue;
                    }

                    let duration_ms = super::api::duration_str_to_ms(
                        track["secondaryText3"].as_str().unwrap_or(""),
                    );
                    duration_map.insert(format!("{album_id}:{track_id}"), duration_ms);
                }
            }
        }

        let tracks: Vec<Track> = parse_search_tracks(&resp, self.search_limit, &duration_map)
            .into_iter()
            .map(Track::new)
            .collect();

        if tracks.is_empty() {
            LoadResult::Empty {}
        } else {
            LoadResult::Search(tracks)
        }
    }
}

#[async_trait]
impl SourcePlugin for AmazonMusicSource {
    fn name(&self) -> &str {
        "amazonmusic"
    }

    fn can_handle(&self, identifier: &str) -> bool {
        self.search_prefixes()
            .iter()
            .any(|p| identifier.starts_with(p))
            || self.domain_re.is_match(identifier)
    }

    fn search_prefixes(&self) -> Vec<&str> {
        vec!["azmsearch:", "amznsearch:"]
    }

    async fn load(
        &self,
        identifier: &str,
        _routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> LoadResult {
        if let Some(prefix) = self
            .search_prefixes()
            .into_iter()
            .find(|p| identifier.starts_with(p))
        {
            return self.load_search(&identifier[prefix.len()..]).await;
        }
        if self.track_re.is_match(identifier) {
            return self.load_track(identifier).await;
        }
        if self.album_re.is_match(identifier) {
            return self.load_album(identifier).await;
        }
        if self.artist_re.is_match(identifier) {
            return self.load_artist(identifier).await;
        }
        if self.user_playlist_re.is_match(identifier) {
            return self.load_community_playlist(identifier).await;
        }
        if self.playlist_re.is_match(identifier) {
            return self.load_playlist(identifier).await;
        }
        LoadResult::Empty {}
    }

    async fn get_track(
        &self,
        _identifier: &str,
        _routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<BoxedTrack> {
        None
    }

    fn get_proxy_config(&self) -> Option<crate::config::HttpProxyConfig> {
        self.proxy.clone()
    }
}
