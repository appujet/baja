use std::sync::Arc;

use futures::future::join_all;
use serde_json::Value;
use tokio::sync::Semaphore;

use super::AppleMusicSource;
use crate::api::tracks::{LoadResult, PlaylistData, PlaylistInfo};

impl AppleMusicSource {
    pub(crate) async fn resolve_track(&self, id: &str) -> LoadResult {
        let path = format!("/catalog/{}/songs/{}", self.country_code, id);

        let data = match self.api_request(&path).await {
            Some(d) => d,
            None => return LoadResult::Empty {},
        };

        if let Some(item) = data.pointer("/data/0") {
            if let Some(track) = self.build_track(item, None) {
                return LoadResult::Track(track);
            }
        }
        LoadResult::Empty {}
    }

    pub(crate) async fn resolve_album(&self, id: &str) -> LoadResult {
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

        let next_url = album
            .pointer("/relationships/tracks/next")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let mut all_items = tracks_data.cloned().unwrap_or_default();
        if next_url.is_some() && (self.album_load_limit == 0 || self.album_load_limit > 1) {
            let extra = self
                .fetch_paginated_tracks(
                    next_url,
                    self.album_load_limit,
                    self.album_page_load_concurrency,
                )
                .await;
            all_items.extend(extra);
        }

        let mut tracks = Vec::new();
        for item in all_items {
            if let Some(track) = self.build_track(&item, artwork.clone()) {
                tracks.push(track);
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
            plugin_info: serde_json::json!({
              "type": "album",
              "url": album.pointer("/attributes/url").and_then(|v| v.as_str()),
              "artworkUrl": artwork,
              "author": album.pointer("/attributes/artistName").and_then(|v| v.as_str()),
              "totalTracks": album.pointer("/attributes/trackCount").and_then(|v| v.as_u64()).unwrap_or(tracks.len() as u64)
            }),
            tracks,
        })
    }

    pub(crate) async fn resolve_playlist(&self, id: &str) -> LoadResult {
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

        let next_url = playlist
            .pointer("/relationships/tracks/next")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let mut all_items = tracks_data.cloned().unwrap_or_default();
        if next_url.is_some() && (self.playlist_load_limit == 0 || self.playlist_load_limit > 1) {
            let extra = self
                .fetch_paginated_tracks(
                    next_url,
                    self.playlist_load_limit,
                    self.playlist_page_load_concurrency,
                )
                .await;
            all_items.extend(extra);
        }

        let mut tracks = Vec::new();
        for item in all_items {
            if let Some(track) = self.build_track(&item, artwork.clone()) {
                tracks.push(track);
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
            plugin_info: serde_json::json!({
              "type": "playlist",
              "url": playlist.pointer("/attributes/url").and_then(|v| v.as_str()),
              "artworkUrl": artwork,
              "author": playlist.pointer("/attributes/curatorName").and_then(|v| v.as_str()),
              "totalTracks": playlist.pointer("/attributes/trackCount").and_then(|v| v.as_u64()).unwrap_or(tracks.len() as u64)
            }),
            tracks,
        })
    }

    pub(crate) async fn resolve_artist(&self, id: &str) -> LoadResult {
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
                if let Some(track) = self.build_track(item, artwork.clone()) {
                    tracks.push(track);
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
            plugin_info: serde_json::json!({
              "type": "artist",
              "url": format!("https://music.apple.com/artist/{}", id),
              "artworkUrl": artwork,
              "author": artist_name,
              "totalTracks": tracks.len()
            }),
            tracks,
        })
    }

    async fn fetch_paginated_tracks(
        &self,
        next_url: Option<String>,
        load_limit: usize,
        concurrency: usize,
    ) -> Vec<Value> {
        let initial_next = match next_url {
            Some(u) => u,
            None => return Vec::new(),
        };

        // If offset is in the URL, we can parallelize
        if initial_next.contains("offset=") {
            let base_url = initial_next
                .split("offset=")
                .next()
                .unwrap_or(&initial_next)
                .to_string();
            let offset: usize = initial_next
                .split("offset=")
                .nth(1)
                .and_then(|s| s.split('&').next())
                .and_then(|s| s.parse().ok())
                .unwrap_or(100);

            // We don't know the total but we can fetch in chunks
            let mut all_items = Vec::new();
            let mut current_offset = offset;
            let mut limit_reached = false;
            let mut pages_fetched = 1; // Initial page is already loaded

            while !limit_reached {
                let mut futs = Vec::new();
                let semaphore = Arc::new(Semaphore::new(concurrency));

                for _ in 0..concurrency {
                    if load_limit > 0 && pages_fetched >= load_limit {
                        limit_reached = true;
                        break;
                    }

                    let url = format!("{}offset={}", base_url, current_offset);
                    let sem = semaphore.clone();
                    let source = self;

                    futs.push(async move {
                        let _permit: tokio::sync::SemaphorePermit<'_> =
                            sem.acquire().await.unwrap();
                        source.api_request(&url).await
                    });

                    current_offset += 100;
                    pages_fetched += 1;
                }

                let results = join_all(futs).await;
                let mut added_on_this_step = 0;

                for res in results {
                    if let Some(data) = res {
                        if let Some(items) = data.get("data").and_then(|v| v.as_array()) {
                            all_items.extend(items.iter().cloned());
                            added_on_this_step += items.len();
                            if items.len() < 100 {
                                limit_reached = true;
                            }
                        } else {
                            limit_reached = true;
                        }
                    } else {
                        limit_reached = true;
                    }
                }

                if added_on_this_step == 0 {
                    break;
                }
            }
            return all_items;
        }

        // Fallback to sequential if offset not found
        let mut next = Some(initial_next);
        let mut all_items = Vec::new();
        let mut pages_fetched = 1;

        while let Some(url) = next {
            if load_limit > 0 && pages_fetched >= load_limit {
                break;
            }

            let data = match self.api_request(&url).await {
                Some(d) => d,
                None => break,
            };

            if let Some(items) = data.get("data").and_then(|v| v.as_array()) {
                all_items.extend(items.iter().cloned());
            }

            next = data
                .get("next")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            pages_fetched += 1;
        }

        all_items
    }
}
