use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use super::YouTubeClient;
use crate::{
    api::tracks::{Track, TrackInfo},
    common::types::AnyResult,
    sources::youtube::{
        cipher::YouTubeCipherManager,
        clients::common::{extract_thumbnail, is_duration, parse_duration},
        oauth::YouTubeOAuth,
    },
};

const CLIENT_NAME: &str = "WEB_REMIX";
const CLIENT_VERSION: &str = "1.20260121.03.00";
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) \
     AppleWebKit/537.36 (KHTML, like Gecko) Chrome/144.0.0.0 Safari/537.36";

const MUSIC_API: &str = "https://music.youtube.com";

pub struct WebRemixClient {
    http: reqwest::Client,
}

impl WebRemixClient {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("Failed to build Music HTTP client");

        Self { http }
    }

    fn build_context(&self, visitor_data: Option<&str>) -> Value {
        let mut client = json!({
            "clientName": CLIENT_NAME,
            "clientVersion": CLIENT_VERSION,
            "userAgent": USER_AGENT,
            "hl": "en",
            "gl": "US"
        });

        if let Some(vd) = visitor_data {
            if let Some(obj) = client.as_object_mut() {
                obj.insert("visitorData".to_string(), vd.into());
            }
        }

        json!({
            "client": client,
            "user": { "lockedSafetyMode": false },
            "request": { "useSsl": true }
        })
    }

    async fn player_request(
        &self,
        video_id: &str,
        visitor_data: Option<&str>,
        signature_timestamp: Option<u32>,
        _oauth: &Arc<YouTubeOAuth>,
    ) -> AnyResult<Value> {
        crate::sources::youtube::clients::common::make_player_request(
            &self.http,
            video_id,
            self.build_context(visitor_data),
            "26", // WEB_REMIX client_id = 26
            CLIENT_VERSION,
            None,
            visitor_data,
            signature_timestamp,
            None,
            None,
            Some(MUSIC_API),
            None,
        )
        .await
    }
}

#[async_trait]
impl YouTubeClient for WebRemixClient {
    fn name(&self) -> &str {
        "MusicWeb"
    }
    fn client_name(&self) -> &str {
        CLIENT_NAME
    }
    fn client_version(&self) -> &str {
        CLIENT_VERSION
    }
    fn user_agent(&self) -> &str {
        USER_AGENT
    }

    async fn search(
        &self,
        query: &str,
        context: &Value,
        oauth: Arc<YouTubeOAuth>,
    ) -> AnyResult<Vec<Track>> {
        let visitor_data = context
            .get("client")
            .and_then(|c| c.get("visitorData"))
            .and_then(|v| v.as_str())
            .or_else(|| context.get("visitorData").and_then(|v| v.as_str()));

        let body = json!({
            "context": self.build_context(visitor_data),
            "query": query,
            "params": "EgWKAQIIAWoQEAMQBBAFEBAQCRAKEBUQEQ%3D%3D"
        });

        let url = format!("{}/youtubei/v1/search?prettyPrint=false", MUSIC_API);

        let mut req = self
            .http
            .post(&url)
            .header("X-YouTube-Client-Name", CLIENT_NAME)
            .header("X-YouTube-Client-Version", CLIENT_VERSION)
            .header("X-Goog-Api-Format-Version", "2")
            .header("Origin", MUSIC_API);

        if let Some(vd) = visitor_data {
            req = req.header("X-Goog-Visitor-Id", vd);
        }

        let req = req.json(&body);

        let _ = oauth;

        let res = req.send().await?;
        if !res.status().is_success() {
            return Err(format!("Music search failed: {}", res.status()).into());
        }

        let response: Value = res.json().await?;
        let mut tracks = Vec::new();

        // Improved navigation for YouTube Music search results
        let tab_content = response
            .get("contents")
            .and_then(|c| c.get("tabbedSearchResultsRenderer"))
            .and_then(|t| t.get("tabs"))
            .and_then(|t| t.get(0))
            .and_then(|t| t.get("tabRenderer"))
            .and_then(|t| t.get("content"));

        let mut shelf_contents = None;
        if let Some(tab) = tab_content {
            if let Some(section_list) = tab.get("sectionListRenderer") {
                if let Some(sections) = section_list.get("contents").and_then(|c| c.as_array()) {
                    for section in sections {
                        if let Some(shelf) = section.get("musicShelfRenderer") {
                            shelf_contents = shelf.get("contents").and_then(|c| c.as_array());
                            if shelf_contents.is_some() {
                                break;
                            }
                        }
                    }
                }
            }
        }

        if let Some(items) = shelf_contents {
            for item in items {
                let renderer = item
                    .get("musicResponsiveListItemRenderer")
                    .or_else(|| item.get("musicTwoColumnItemRenderer"));

                if let Some(renderer) = renderer {
                    // Extract video ID safely
                    let id = renderer
                        .get("playlistItemData")
                        .and_then(|d| d.get("videoId"))
                        .and_then(|v| v.as_str())
                        .or_else(|| {
                            renderer
                                .get("doubleTapCommand")
                                .and_then(|c| c.get("watchEndpoint"))
                                .and_then(|w| w.get("videoId"))
                                .and_then(|v| v.as_str())
                        })
                        .or_else(|| renderer.get("videoId").and_then(|v| v.as_str()));

                    if let Some(id) = id {
                        // Improved title extraction
                        let title = renderer
                            .get("flexColumns")
                            .and_then(|c| c.get(0))
                            .and_then(|c| c.get("musicResponsiveListItemFlexColumnRenderer"))
                            .and_then(|r| r.get("text"))
                            .and_then(|t| t.get("runs"))
                            .and_then(|r| r.get(0))
                            .and_then(|r| r.get("text"))
                            .and_then(|t| t.as_str())
                            .unwrap_or("Unknown Title");

                        // Improved author and length extraction from runs
                        let mut author = "Unknown Artist".to_string();
                        let mut length_ms = 0u64;

                        // Check flex columns for author and duration
                        if let Some(flex_cols) =
                            renderer.get("flexColumns").and_then(|c| c.as_array())
                        {
                            // Column 1 is usually Artist
                            if flex_cols.len() > 1 {
                                if let Some(a) = flex_cols[1]
                                    .get("musicResponsiveListItemFlexColumnRenderer")
                                    .and_then(|r| r.get("text"))
                                    .and_then(|t| t.get("runs"))
                                    .and_then(|r| r.get(0))
                                    .and_then(|r| r.get("text"))
                                    .and_then(|t| t.as_str())
                                {
                                    author = a.to_string();
                                }
                            }

                            // Scan all columns for duration string
                            for col in flex_cols {
                                if let Some(runs) = col
                                    .get("musicResponsiveListItemFlexColumnRenderer")
                                    .and_then(|r| r.get("text"))
                                    .and_then(|t| t.get("runs"))
                                    .and_then(|r| r.as_array())
                                {
                                    for run in runs {
                                        if let Some(text) = run.get("text").and_then(|t| t.as_str())
                                        {
                                            if is_duration(text) {
                                                length_ms = parse_duration(text);
                                                break;
                                            }
                                        }
                                    }
                                }
                                if length_ms > 0 {
                                    break;
                                }
                            }
                        }

                        // Fallback author if still default
                        if author == "Unknown Artist" {
                            if let Some(subtitle_runs) = renderer
                                .get("subtitle")
                                .and_then(|s| s.get("runs"))
                                .and_then(|r| r.as_array())
                            {
                                if !subtitle_runs.is_empty() {
                                    if let Some(a) =
                                        subtitle_runs[0].get("text").and_then(|t| t.as_str())
                                    {
                                        author = a.to_string();
                                    }
                                }
                            }
                        }

                        // Artwork URL extraction
                        let artwork_url = extract_thumbnail(renderer, Some(id));

                        let info = TrackInfo {
                            identifier: id.to_string(),
                            is_seekable: true,
                            title: title.to_string(),
                            author,
                            length: length_ms,
                            is_stream: false,
                            uri: Some(format!("https://music.youtube.com/watch?v={}", id)),
                            source_name: "youtube".to_string(),
                            isrc: None,
                            artwork_url,
                            position: 0,
                        };
                        tracks.push(Track::new(info));
                    }
                }
            }
        }

        Ok(tracks)
    }

    async fn get_track_info(
        &self,
        track_id: &str,
        context: &Value,
        oauth: Arc<YouTubeOAuth>,
    ) -> AnyResult<Option<Track>> {
        let visitor_data = context
            .get("client")
            .and_then(|c| c.get("visitorData"))
            .and_then(|v| v.as_str())
            .or_else(|| context.get("visitorData").and_then(|v| v.as_str()));

        let body = self
            .player_request(track_id, visitor_data, None, &oauth)
            .await?;

        let playability = body
            .get("playabilityStatus")
            .and_then(|p| p.get("status"))
            .and_then(|s| s.as_str())
            .unwrap_or("UNKNOWN");

        if playability != "OK" {
            return Ok(None);
        }

        let vd = body.get("videoDetails");
        let title = vd
            .and_then(|v| v.get("title"))
            .and_then(|t| t.as_str())
            .unwrap_or("Unknown Title");
        let author = vd
            .and_then(|v| v.get("author"))
            .and_then(|a| a.as_str())
            .unwrap_or("Unknown Artist");
        let length_secs = vd
            .and_then(|v| v.get("lengthSeconds"))
            .and_then(|l| l.as_str())
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);

        let info = TrackInfo {
            identifier: track_id.to_string(),
            is_seekable: true,
            title: title.to_string(),
            author: author.to_string(),
            length: length_secs * 1000,
            is_stream: false,
            uri: Some(format!("https://music.youtube.com/watch?v={}", track_id)),
            source_name: "youtube".to_string(),
            isrc: None,
            artwork_url: extract_thumbnail(&vd.cloned().unwrap_or(Value::Null), Some(track_id)),
            position: 0,
        };

        Ok(Some(Track::new(info)))
    }

    async fn get_playlist(
        &self,
        playlist_id: &str,
        context: &Value,
        oauth: Arc<YouTubeOAuth>,
    ) -> AnyResult<Option<(Vec<Track>, String)>> {
        let visitor_data = context
            .get("client")
            .and_then(|c| c.get("visitorData"))
            .and_then(|v| v.as_str())
            .or_else(|| context.get("visitorData").and_then(|v| v.as_str()));

        // Resolve a playlist/album via 'next' endpoint (common for YT Music)
        let body = json!({
            "context": self.build_context(visitor_data),
            "playlistId": playlist_id,
            "isAudioOnly": true
        });

        let url = format!("{}/youtubei/v1/next?prettyPrint=false", MUSIC_API);

        let mut req = self
            .http
            .post(&url)
            .header("X-YouTube-Client-Name", CLIENT_NAME)
            .header("X-YouTube-Client-Version", CLIENT_VERSION);

        if let Some(vd) = visitor_data {
            req = req.header("X-Goog-Visitor-Id", vd);
        }

        let req = req.json(&body);

        let _ = oauth;

        let res = req.send().await?;
        if !res.status().is_success() {
            return Ok(None);
        }

        let response: Value = res.json().await?;

        Ok(crate::sources::youtube::extractor::extract_from_next(
            &response, "youtube",
        ))
    }

    async fn resolve_url(
        &self,
        _url: &str,
        _context: &Value,
        _oauth: Arc<YouTubeOAuth>,
    ) -> AnyResult<Option<Track>> {
        tracing::debug!("{} client does not support resolve_url", self.name());
        Ok(None)
    }

    async fn get_track_url(
        &self,
        _track_id: &str,
        _context: &Value,
        _cipher_manager: Arc<YouTubeCipherManager>,
        _oauth: Arc<YouTubeOAuth>,
    ) -> AnyResult<Option<String>> {
        tracing::debug!("{} client does not provide direct track URLs", self.name());
        Ok(None)
    }
}
