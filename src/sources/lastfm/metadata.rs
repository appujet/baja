use super::{LastFMSource, helpers::{get_json, unescape_html, parse_duration_to_ms}};
use crate::protocol::tracks::{LoadResult, PlaylistData, PlaylistInfo, Track, TrackInfo};
use regex::Regex;

impl LastFMSource {
    pub async fn resolve_url(&self, url: &str) -> LoadResult {
        let caps = match crate::sources::lastfm::path_regex().captures(url) {
            Some(c) => c,
            None => {
                tracing::debug!("Last.fm: URL path failed to match regex: {}", url);
                return LoadResult::Empty {};
            }
        };
 
        let type_ = caps.get(1).map(|m| m.as_str()).unwrap_or("");
        let p1 = urlencoding::decode(caps.get(2).map(|m| m.as_str()).unwrap_or(""))
            .unwrap_or_default()
            .to_string();
        let p2 = urlencoding::decode(caps.get(3).map(|m| m.as_str()).unwrap_or(""))
            .unwrap_or_default()
            .to_string();
        let p3 = urlencoding::decode(caps.get(4).map(|m| m.as_str()).unwrap_or(""))
            .unwrap_or_default()
            .to_string();
 
        if type_ == "user" {
            return self.resolve_user(&p1, url).await;
        }
 
        if p3.is_empty() {
            if p2 == "_" || p2.is_empty() {
                self.resolve_artist(&p1, url).await
            } else {
                self.resolve_album(&p1, &p2, url).await
            }
        } else {
            self.resolve_track(&p1, &p3, url).await
        }
    }

    pub async fn resolve_track(&self, artist: &str, title: &str, url: &str) -> LoadResult {
        let mut artwork_url = None;
        let mut length = 0;
 
        if let Some(ref key) = self.api_key {
            let api_url = format!(
                "https://ws.audioscrobbler.com/2.0/?method=track.getInfo&api_key={}&artist={}&track={}&format=json",
                key,
                urlencoding::encode(artist),
                urlencoding::encode(title)
            );
 
            if let Some(json) = get_json(&self.http, &api_url).await {
                artwork_url = json["track"]["album"]["image"]
                    .as_array()
                    .or_else(|| json["track"]["image"].as_array())
                    .and_then(|images| images.last())
                    .and_then(|img| img["#text"].as_str())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.replace("/34s/", "/300x300/"));

                length = json["track"]["duration"]
                    .as_str()
                    .and_then(|s| s.parse::<u64>().ok())
                    .or_else(|| json["track"]["duration"].as_u64())
                    .unwrap_or(0);
            }
        }
 
        if artwork_url.is_none() || length == 0 {
            if let Ok(res) = self.http.get(url).send().await {
                if let Ok(body) = res.text().await {
                    if artwork_url.is_none() {
                        if let Some(caps) = Regex::new(r#"(?i)<img[^>]*?class="[^"]*header-new-background-image[^"]*"[^>]*?src="([^"]+)""#)
                            .ok()
                            .and_then(|r| r.captures(&body)) {
                            artwork_url = caps.get(1).map(|m| m.as_str().replace("/64s/", "/300x300/"));
                        }
                    }
                    if length == 0 {
                        if let Some(caps) = Regex::new(r#"(?i)<dt[^>]*?>\s*Length\s*</dt>\s*<dd[^>]*?class="[^"]*catalogue-metadata-description[^"]*"[^>]*?>\s*(\d+:\d+(?::\d+)?)\s*</dd>"#)
                            .ok()
                            .and_then(|r| r.captures(&body)) {
                            length = parse_duration_to_ms(caps.get(1).map(|m| m.as_str()).unwrap_or("0:00"));
                        }
                    }
                }
            }
        }
 
        let canonical_url = crate::sources::lastfm::construct_track_url(artist, title);

        LoadResult::Track(Track::new(TrackInfo {
            identifier: canonical_url.clone(),
            is_seekable: true,
            author: artist.to_owned(),
            title: title.to_owned(),
            length,
            uri: Some(canonical_url),
            artwork_url,
            source_name: "lastfm".to_owned(),
            ..Default::default()
        }))
    }

    pub async fn resolve_album(&self, artist: &str, album: &str, url: &str) -> LoadResult {
        if let Some(ref key) = self.api_key {
            let api_url = format!(
                "https://ws.audioscrobbler.com/2.0/?method=album.getinfo&api_key={}&artist={}&album={}&format=json",
                key,
                urlencoding::encode(artist),
                urlencoding::encode(album)
            );

            if let Some(json) = get_json(&self.http, &api_url).await {
                if let Some(tracks) = json["album"]["tracks"]["track"].as_array() {
                    let artwork_url = json["album"]["image"]
                        .as_array()
                        .and_then(|images| images.last())
                        .and_then(|img| img["#text"].as_str())
                        .filter(|s| !s.is_empty())
                        .map(|s| s.replace("/34s/", "/300x300/"));

                    let mut results = Vec::new();
                    for t in tracks {
                        let title = t["name"].as_str().unwrap_or("Unknown").to_owned();
                        let t_url = crate::sources::lastfm::construct_track_url(artist, &title);
                        let length = t["duration"]
                            .as_str()
                            .and_then(|s| s.parse::<u64>().ok())
                            .or_else(|| t["duration"].as_u64())
                            .unwrap_or(0) * 1000;

                        results.push(Track::new(TrackInfo {
                            identifier: t_url.clone(),
                            is_seekable: true,
                            author: artist.to_owned(),
                            title,
                            length,
                            uri: Some(t_url),
                            artwork_url: artwork_url.clone(),
                            source_name: "lastfm".to_owned(),
                            ..Default::default()
                        }));
                    }

                    return LoadResult::Playlist(PlaylistData {
                        info: PlaylistInfo {
                            name: format!("{} - {}", artist, album),
                            selected_track: -1,
                        },
                        plugin_info: serde_json::json!({}),
                        tracks: results,
                    });
                }
            }
        }

        let body = match self.http.get(url).send().await {
            Ok(r) => r.text().await.unwrap_or_else(|e| {
                tracing::debug!("Last.fm: failed to get response text for album {}: {}", url, e);
                Default::default()
            }),
            Err(e) => {
                tracing::debug!("Last.fm: album scraping request failed for {}: {}", url, e);
                return LoadResult::Empty {};
            }
        };

        let mut results = Vec::new();
        for caps in crate::sources::lastfm::search_regex().captures_iter(&body) {
            let artwork_url = caps.get(1).map(|m| m.as_str().replace("/64s/", "/300x300/"));
            let title = unescape_html(caps.get(2).map(|m| m.as_str()).unwrap_or("Unknown"));
            let full_url = crate::sources::lastfm::construct_track_url(artist, &title);

            results.push(Track::new(TrackInfo {
                identifier: full_url.clone(),
                is_seekable: true,
                author: artist.to_owned(),
                title: title.to_owned(),
                uri: Some(full_url),
                artwork_url,
                source_name: "lastfm".to_owned(),
                ..Default::default()
            }));
        }

        if results.is_empty() {
            tracing::debug!("Last.fm: album/artist search yielded no tracks on page {}, trying track fallback", url);
            self.resolve_track(artist, album, url).await
        } else {
            LoadResult::Playlist(PlaylistData {
                info: PlaylistInfo {
                    name: format!("{} - {}", artist, album),
                    selected_track: -1,
                },
                plugin_info: serde_json::json!({}),
                tracks: results,
            })
        }
    }

    pub async fn resolve_artist(&self, artist: &str, url: &str) -> LoadResult {
        if let Some(ref key) = self.api_key {
            let api_url = format!(
                "https://ws.audioscrobbler.com/2.0/?method=artist.gettoptracks&api_key={}&artist={}&limit=50&format=json",
                key,
                urlencoding::encode(artist)
            );
 
            if let Some(json) = get_json(&self.http, &api_url).await {
                if let Some(tracks) = json["toptracks"]["track"].as_array() {
                    let mut results = Vec::new();
                    for t in tracks {
                        let title = t["name"].as_str().unwrap_or("Unknown").to_owned();
                        let t_url = crate::sources::lastfm::construct_track_url(artist, &title);
                        let artwork_url = t["image"]
                            .as_array()
                            .and_then(|images| images.last())
                            .and_then(|img| img["#text"].as_str())
                            .filter(|s| !s.is_empty())
                            .map(|s| s.replace("/34s/", "/300x300/"));
 
                        let length = t["duration"]
                            .as_str()
                            .and_then(|s| s.parse::<u64>().ok())
                            .or_else(|| t["duration"].as_u64())
                            .unwrap_or(0) * 1000;

                        results.push(Track::new(TrackInfo {
                            identifier: t_url.clone(),
                            is_seekable: true,
                            author: artist.to_owned(),
                            title,
                            length,
                            uri: Some(t_url),
                            artwork_url,
                            source_name: "lastfm".to_owned(),
                            ..Default::default()
                        }));
                    }
 
                    return LoadResult::Playlist(PlaylistData {
                        info: PlaylistInfo {
                            name: format!("{} - Top Tracks", artist),
                            selected_track: -1,
                        },
                        plugin_info: serde_json::json!({}),
                        tracks: results,
                    });
                }
            }
        }
 
        let body = match self.http.get(url).send().await {
            Ok(r) => r.text().await.unwrap_or_else(|e| {
                tracing::debug!("Last.fm: failed to get response text for artist tracks {}: {}", url, e);
                Default::default()
            }),
            Err(e) => {
                tracing::debug!("Last.fm: artist tracks scraping request failed for {}: {}", url, e);
                return LoadResult::Empty {};
            }
        };
 
        let mut results = Vec::new();
        for caps in crate::sources::lastfm::search_regex().captures_iter(&body) {
            let artwork_url = caps.get(1).map(|m| m.as_str().replace("/64s/", "/300x300/"));
            let title = unescape_html(caps.get(2).map(|m| m.as_str()).unwrap_or("Unknown"));
            let full_url = crate::sources::lastfm::construct_track_url(artist, &title);
 
            results.push(Track::new(TrackInfo {
                identifier: full_url.clone(),
                is_seekable: true,
                author: artist.to_owned(),
                title: title.to_owned(),
                uri: Some(full_url),
                artwork_url,
                source_name: "lastfm".to_owned(),
                ..Default::default()
            }));
        }
 
        if results.is_empty() {
            LoadResult::Empty {}
        } else {
            LoadResult::Playlist(PlaylistData {
                info: PlaylistInfo {
                    name: format!("{} - Top Tracks", artist),
                    selected_track: -1,
                },
                plugin_info: serde_json::json!({}),
                tracks: results,
            })
        }
    }
 
    pub async fn resolve_user(&self, username: &str, url: &str) -> LoadResult {
        if let Some(ref key) = self.api_key {
            let api_url = format!(
                "https://ws.audioscrobbler.com/2.0/?method=user.gettoptracks&user={}&api_key={}&limit=50&format=json",
                urlencoding::encode(username),
                key
            );
 
            if let Some(json) = get_json(&self.http, &api_url).await {
                if let Some(tracks) = json["toptracks"]["track"].as_array() {
                    let mut results = Vec::new();
                    for t in tracks {
                        let title = t["name"].as_str().unwrap_or("Unknown").to_owned();
                        let artist = t["artist"]["name"].as_str().unwrap_or("Unknown").to_owned();
                        let t_url = crate::sources::lastfm::construct_track_url(&artist, &title);
                        let artwork_url = t["image"]
                            .as_array()
                            .and_then(|images| images.last())
                            .and_then(|img| img["#text"].as_str())
                            .filter(|s| !s.is_empty())
                            .map(|s| s.replace("/34s/", "/300x300/"));
 
                        let length = t["duration"]
                            .as_str()
                            .and_then(|s| s.parse::<u64>().ok())
                            .or_else(|| t["duration"].as_u64())
                            .unwrap_or(0) * 1000;

                        results.push(Track::new(TrackInfo {
                            identifier: t_url.clone(),
                            is_seekable: true,
                            author: artist,
                            title,
                            length,
                            uri: Some(t_url),
                            artwork_url,
                            source_name: "lastfm".to_owned(),
                            ..Default::default()
                        }));
                    }
 
                    return LoadResult::Playlist(PlaylistData {
                        info: PlaylistInfo {
                            name: format!("{}'s Top Tracks", username),
                            selected_track: -1,
                        },
                        plugin_info: serde_json::json!({}),
                        tracks: results,
                    });
                }
            }
        }
 
        let body = match self.http.get(url).send().await {
            Ok(r) => r.text().await.unwrap_or_else(|e| {
                tracing::debug!("Last.fm: failed to get response text for user profile {}: {}", url, e);
                Default::default()
            }),
            Err(e) => {
                tracing::debug!("Last.fm: user profile scraping request failed for {}: {}", url, e);
                return LoadResult::Empty {};
            }
        };
 
        let mut results = Vec::new();
        for caps in crate::sources::lastfm::search_regex().captures_iter(&body) {
            let artwork_url = caps.get(1).map(|m| m.as_str().replace("/64s/", "/300x300/"));
            let title = unescape_html(caps.get(2).map(|m| m.as_str()).unwrap_or("Unknown"));
            let artist = unescape_html(caps.get(4).map(|m| m.as_str()).unwrap_or("Unknown"));
            let full_url = crate::sources::lastfm::construct_track_url(&artist, &title);
 
            results.push(Track::new(TrackInfo {
                identifier: full_url.clone(),
                is_seekable: true,
                author: artist,
                title,
                uri: Some(full_url),
                artwork_url,
                source_name: "lastfm".to_owned(),
                ..Default::default()
            }));
        }
 
        if results.is_empty() {
            LoadResult::Empty {}
        } else {
            LoadResult::Playlist(PlaylistData {
                info: PlaylistInfo {
                    name: format!("{}'s Recent Tracks", username),
                    selected_track: -1,
                },
                plugin_info: serde_json::json!({}),
                tracks: results,
            })
        }
    }
}
