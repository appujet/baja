use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::{
    configs::Config,
    protocol::tracks::{LoadResult, PlaylistData, PlaylistInfo, Track, TrackInfo},
    sources::{SourcePlugin, plugin::BoxedTrack},
};

const BASE_URL: &str = "https://api.anghami.com/gateway.php";

pub struct AnghamiSource {
    client: Arc<reqwest::Client>,
    udid: String,
    search_limit: usize,
    search_prefixes: Vec<String>,
    url_regex: regex::Regex,
}

impl AnghamiSource {
    pub fn new(config: &Config, client: Arc<reqwest::Client>) -> Result<Self, String> {
        let ag_config = config.anghami.clone().unwrap_or_default();

        let udid = uuid::Uuid::new_v4().simple().to_string();

        Ok(Self {
      client,
      udid,
      search_limit: ag_config.search_limit,
      search_prefixes: vec!["agsearch:".to_string()],
      url_regex: regex::Regex::new(
        r"^https?://(?:play\.|www\.)?anghami\.com/(?P<type>song|album|playlist|artist)/(?P<id>[0-9]+)"
      ).unwrap(),
    })
    }

    fn unix_ts(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    async fn api_request(&self, params: Vec<(&str, &str)>) -> Option<Value> {
        let mut url = reqwest::Url::parse(BASE_URL).ok()?;
        {
            let mut q = url.query_pairs_mut();
            for (k, v) in &params {
                q.append_pair(k, v);
            }
        }

        let resp = self
            .base_request(self.client.get(url))
            .header("X-ANGH-UDID", &self.udid)
            .header("X-ANGH-TS", self.unix_ts().to_string())
            .send()
            .await
            .ok()?;

        if !resp.status().is_success() {
            return None;
        }

        resp.json::<Value>().await.ok()
    }

    fn build_artwork_url(json: &Value) -> Option<String> {
        let art_id = json["coverArt"]
            .as_str()
            .or_else(|| json["AlbumArt"].as_str())
            .or_else(|| json["cover"].as_str())?;
        if art_id.is_empty() {
            return None;
        }
        Some(format!(
            "https://artwork.anghcdn.co/?id={}&size=640",
            art_id
        ))
    }

    fn parse_track(&self, json: &Value) -> Option<Track> {
        let id = match json["id"].as_str() {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => match json["id"].as_i64() {
                Some(n) => n.to_string(),
                None => return None,
            },
        };

        let title = json["title"]
            .as_str()
            .or_else(|| json["name"].as_str())
            .filter(|s| !s.is_empty())?
            .to_string();

        let author = json["artist"]
            .as_str()
            .or_else(|| json["artistName"].as_str())
            .unwrap_or("Unknown Artist")
            .to_string();

        let duration_secs = json["duration"]
            .as_f64()
            .or_else(|| {
                json["duration"]
                    .as_str()
                    .and_then(|s| s.parse::<f64>().ok())
            })
            .unwrap_or(0.0);
        let length = (duration_secs * 1000.0).round() as u64;
        let artwork_url = Self::build_artwork_url(json);
        let uri = format!("https://play.anghami.com/song/{}", id);

        Some(Track::new(TrackInfo {
            identifier: id,
            is_seekable: true,
            author,
            length,
            is_stream: false,
            position: 0,
            title,
            uri: Some(uri),
            artwork_url,
            isrc: None,
            source_name: "anghami".to_string(),
        }))
    }

    fn extract_tracks(&self, body: &Value) -> Vec<Track> {
        if let Some(songbuffers) = body["songbuffers"].as_array() {
            let mut song_map: std::collections::HashMap<String, Track> =
                std::collections::HashMap::new();
            for buffer_base64 in songbuffers {
                if let Some(s) = buffer_base64.as_str() {
                    if let Ok(decoded) =
                        base64::Engine::decode(&base64::prelude::BASE64_STANDARD, s.as_bytes())
                    {
                        let songs = super::reader::decode_song_batch(&decoded);
                        for (id, track_info) in songs {
                            song_map.insert(id, Track::new(track_info));
                        }
                    }
                }
            }

            if !song_map.is_empty() {
                let order_str = [
                    body["songorder"].as_str(),
                    body["_attributes"]["songorder"].as_str(),
                    body["playlist"]["songorder"].as_str(),
                    body["album"]["songorder"].as_str(),
                ]
                .iter()
                .find_map(|o| *o);

                if let Some(order) = order_str {
                    let tracks: Vec<Track> = order
                        .split(',')
                        .filter_map(|id| song_map.remove(id.trim()))
                        .collect();
                    if !tracks.is_empty() {
                        return tracks;
                    }
                }

                return song_map.into_values().collect();
            }
        }

        if let Some(sections) = body["sections"].as_array() {
            for section in sections {
                let type_ = section["type"].as_str().unwrap_or("");
                let group = section["group"].as_str().unwrap_or("");
                if type_ == "song" || group == "songs" || group == "album_songs" {
                    if let Some(data) = section["data"].as_array() {
                        let tracks: Vec<Track> =
                            data.iter().filter_map(|s| self.parse_track(s)).collect();
                        if !tracks.is_empty() {
                            return tracks;
                        }
                    }
                }
            }
        }

        for songs in &[
            &body["songs"],
            &body["playlist"]["songs"],
            &body["album"]["songs"],
        ] {
            if songs.is_null() {
                continue;
            }
            let mut song_map: std::collections::HashMap<String, &Value> =
                std::collections::HashMap::new();
            if let Some(obj) = songs.as_object() {
                for (_k, v) in obj {
                    let s = if !v["_attributes"].is_null() {
                        &v["_attributes"]
                    } else {
                        v
                    };
                    let id = s["id"]
                        .as_str()
                        .map(|s| s.to_string())
                        .or_else(|| s["id"].as_i64().map(|n| n.to_string()));
                    if let Some(id) = id {
                        song_map.insert(id, s);
                    }
                }
            }
            if song_map.is_empty() {
                continue;
            }

            let order_str = [
                body["songorder"].as_str(),
                body["_attributes"]["songorder"].as_str(),
                body["playlist"]["songorder"].as_str(),
                body["album"]["songorder"].as_str(),
            ]
            .iter()
            .find_map(|o| *o);

            let tracks: Vec<Track> = if let Some(order) = order_str {
                order
                    .split(',')
                    .filter_map(|id| song_map.get(id.trim()))
                    .filter_map(|s| self.parse_track(s))
                    .collect()
            } else {
                song_map
                    .values()
                    .filter_map(|s| self.parse_track(s))
                    .collect()
            };

            if !tracks.is_empty() {
                return tracks;
            }
        }

        body["data"]
            .as_array()
            .map(|data| data.iter().filter_map(|s| self.parse_track(s)).collect())
            .unwrap_or_default()
    }

    fn collection_title(body: &Value, type_: &str, default: &str) -> String {
        let mut candidates = vec![
            body["title"].as_str(),
            body["name"].as_str(),
            body["playlist_name"].as_str(),
            body["album_name"].as_str(),
            body["albumTitle"].as_str(),
            body["playlistTitle"].as_str(),
            body["album_info"]["title"].as_str(),
            body["playlist_info"]["title"].as_str(),
        ];

        for t in &["album", "playlist", type_] {
            candidates.push(body[*t]["title"].as_str());
            candidates.push(body[*t]["name"].as_str());
            candidates.push(body[*t]["album_name"].as_str());
            candidates.push(body[*t]["playlist_name"].as_str());
            candidates.push(body[*t]["albumTitle"].as_str());
            candidates.push(body[*t]["playlistTitle"].as_str());
            candidates.push(body[*t]["_attributes"]["title"].as_str());
            candidates.push(body[*t]["_attributes"]["name"].as_str());
            candidates.push(body[*t]["_attributes"]["album_name"].as_str());
            candidates.push(body[*t]["_attributes"]["playlist_name"].as_str());
        }

        candidates.push(body["_attributes"]["title"].as_str());
        candidates.push(body["_attributes"]["name"].as_str());

        if let Some(title) = candidates
            .into_iter()
            .find_map(|s| s.filter(|v| !v.is_empty()).map(str::to_string))
        {
            return title;
        }

        if let Some(sections) = body["sections"].as_array() {
            for sec in sections {
                if let Some(t) = sec["title"]
                    .as_str()
                    .or(sec["name"].as_str())
                    .filter(|s| !s.is_empty())
                {
                    return t.to_string();
                }
            }
        }

        default.to_string()
    }

    async fn get_search(&self, query: &str) -> LoadResult {
        if query.is_empty() {
            return LoadResult::Empty {};
        }

        let body = match self
            .api_request(vec![
                ("type", "GETtabsearch"),
                ("query", query),
                ("web2", "true"),
                ("language", "en"),
                ("output", "json"),
            ])
            .await
        {
            Some(b) => b,
            None => return LoadResult::Empty {},
        };

        let tracks: Vec<Track> = body["sections"]
            .as_array()
            .and_then(|secs| {
                secs.iter().find(|s| {
                    s["type"].as_str() == Some("genericitem")
                        && s["group"].as_str() == Some("songs")
                })
            })
            .and_then(|s| s["data"].as_array())
            .map(|data| {
                data.iter()
                    .take(self.search_limit)
                    .filter_map(|item| self.parse_track(item))
                    .collect()
            })
            .unwrap_or_default();

        if tracks.is_empty() {
            return LoadResult::Empty {};
        }
        LoadResult::Search(tracks)
    }

    async fn get_song(&self, id: &str) -> LoadResult {
        if let Some(body) = self
            .api_request(vec![
                ("type", "GETsongdata"),
                ("songId", id),
                ("output", "jsonhp"),
            ])
            .await
        {
            if body["status"].as_str() == Some("ok") {
                if let Some(track) = self.parse_track(&body) {
                    return LoadResult::Track(track);
                }
            }
        }

        if let Some(body) = self
            .api_request(vec![
                ("type", "GETtabsearch"),
                ("query", id),
                ("web2", "true"),
                ("language", "en"),
                ("output", "json"),
            ])
            .await
        {
            if let Some(sections) = body["sections"].as_array() {
                for section in sections {
                    if let Some(data) = section["data"].as_array() {
                        let song = data.iter().find(|s| {
                            s["id"].as_str() == Some(id)
                                || s["id"].as_i64().map(|n| n.to_string()).as_deref() == Some(id)
                        });
                        if let Some(s) = song {
                            if let Some(track) = self.parse_track(s) {
                                return LoadResult::Track(track);
                            }
                        }
                    }
                }
            }
        }

        LoadResult::Empty {}
    }

    async fn get_album(&self, id: &str) -> LoadResult {
        for buffered in &[false, true] {
            let mut params = vec![
                ("type", "GETalbumdata"),
                ("albumId", id),
                ("web2", "true"),
                ("language", "en"),
                ("output", "json"),
            ];
            if *buffered {
                params.push(("buffered", "1"));
            }

            let body = match self.api_request(params).await {
                Some(b) if b["error"].is_null() => b,
                _ => continue,
            };

            let tracks = self.extract_tracks(&body);
            if tracks.is_empty() {
                continue;
            }

            let mut name = Self::collection_title(&body, "album", "Unknown Album");
            if name == "Unknown Album" {
                if let Some(first_album) = body["data"]
                    .as_array()
                    .and_then(|a| a.first())
                    .and_then(|t| t["album"].as_str().or(t["albumName"].as_str()))
                {
                    name = first_album.to_string();
                } else if let Some(sections) = body["sections"].as_array() {
                    for sec in sections {
                        if let Some(first_album) = sec["data"]
                            .as_array()
                            .and_then(|a| a.first())
                            .and_then(|t| t["album"].as_str().or(t["albumName"].as_str()))
                        {
                            name = first_album.to_string();
                            break;
                        }
                    }
                }
            }
            let artwork_url = tracks.first().and_then(|t| t.info.artwork_url.clone());
            let author = tracks.first().map(|t| t.info.author.clone());

            return LoadResult::Playlist(PlaylistData {
                info: PlaylistInfo {
                    name,
                    selected_track: -1,
                },
                plugin_info: json!({
                  "type": "album",
                  "url": format!("https://play.anghami.com/album/{}", id),
                  "artworkUrl": artwork_url,
                  "author": author,
                  "totalTracks": tracks.len()
                }),
                tracks,
            });
        }

        LoadResult::Empty {}
    }

    async fn get_playlist(&self, id: &str) -> LoadResult {
        for buffered in &[false, true] {
            let mut params = vec![
                ("type", "GETplaylistdata"),
                ("playlistId", id),
                ("web2", "true"),
                ("language", "en"),
                ("output", "json"),
            ];
            if *buffered {
                params.push(("buffered", "1"));
            }

            let body = match self.api_request(params).await {
                Some(b) if b["error"].is_null() => b,
                _ => continue,
            };

            let tracks = self.extract_tracks(&body);
            if tracks.is_empty() {
                continue;
            }

            let mut name = Self::collection_title(&body, "playlist", "Unknown Playlist");
            if name == "Unknown Playlist" {
                if let Some(alt_name) = body["playlist"]["name"]
                    .as_str()
                    .or(body["playlist"]["title"].as_str())
                {
                    name = alt_name.to_string();
                }
            }
            let artwork_url = tracks.first().and_then(|t| t.info.artwork_url.clone());

            return LoadResult::Playlist(PlaylistData {
                info: PlaylistInfo {
                    name,
                    selected_track: -1,
                },
                plugin_info: json!({
                  "type": "playlist",
                  "url": format!("https://play.anghami.com/playlist/{}", id),
                  "artworkUrl": artwork_url,
                  "totalTracks": tracks.len()
                }),
                tracks,
            });
        }

        LoadResult::Empty {}
    }

    async fn get_artist(&self, id: &str) -> LoadResult {
        let body = match self
            .api_request(vec![
                ("type", "GETartistprofile"),
                ("artistId", id),
                ("web2", "true"),
                ("language", "en"),
                ("output", "json"),
            ])
            .await
        {
            Some(b) => b,
            None => return LoadResult::Empty {},
        };

        let tracks: Vec<Track> = if let Some(sections) = body["sections"].as_array() {
            sections
                .iter()
                .find(|s| {
                    s["group"].as_str() == Some("songs") || s["type"].as_str() == Some("song")
                })
                .and_then(|s| s["data"].as_array())
                .map(|data| data.iter().filter_map(|s| self.parse_track(s)).collect())
                .unwrap_or_default()
        } else {
            body["data"]
                .as_array()
                .map(|data| data.iter().filter_map(|s| self.parse_track(s)).collect())
                .unwrap_or_default()
        };

        if tracks.is_empty() {
            return LoadResult::Empty {};
        }

        let name = body["name"]
            .as_str()
            .or_else(|| body["title"].as_str())
            .unwrap_or("Unknown Artist")
            .to_string();
        let artwork_url = tracks.first().and_then(|t| t.info.artwork_url.clone());

        LoadResult::Playlist(PlaylistData {
            info: PlaylistInfo {
                name: format!("{}'s Top Tracks", name),
                selected_track: -1,
            },
            plugin_info: json!({
              "type": "artist",
              "url": format!("https://play.anghami.com/artist/{}", id),
              "artworkUrl": artwork_url,
              "author": name,
              "totalTracks": tracks.len()
            }),
            tracks,
        })
    }

    pub fn base_request(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        builder
            .header(reqwest::header::USER_AGENT, "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
            .header("Referer", "https://play.anghami.com/")
            .header("Origin", "https://play.anghami.com")
    }
}

#[async_trait]
impl SourcePlugin for AnghamiSource {
    fn name(&self) -> &str {
        "anghami"
    }

    fn can_handle(&self, identifier: &str) -> bool {
        self.search_prefixes
            .iter()
            .any(|p| identifier.starts_with(p))
            || self.url_regex.is_match(identifier)
    }

    fn search_prefixes(&self) -> Vec<&str> {
        self.search_prefixes.iter().map(|s| s.as_str()).collect()
    }

    fn is_mirror(&self) -> bool {
        true
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
            return self.get_search(&identifier[prefix.len()..]).await;
        }

        if let Some(caps) = self.url_regex.captures(identifier) {
            let type_ = caps.name("type").map(|m| m.as_str()).unwrap_or("");
            let id = caps.name("id").map(|m| m.as_str()).unwrap_or("");

            return match type_ {
                "song" => self.get_song(id).await,
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
        _identifier: &str,
        _routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<BoxedTrack> {
        None
    }
}
