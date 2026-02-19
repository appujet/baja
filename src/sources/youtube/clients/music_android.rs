use super::YouTubeClient;
use super::common::{
    extract_thumbnail, is_duration, parse_duration, resolve_format_url, select_best_audio_format,
};
use crate::api::tracks::{Track, TrackInfo};
use crate::sources::youtube::cipher::YouTubeCipherManager;
use crate::sources::youtube::oauth::YouTubeOAuth;
use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use serde_json::{Value, json};
use std::sync::Arc;

/// Using ANDROID_MUSIC as it mirrors the NodeLink-dev Music.js implementation.
const CLIENT_NAME: &str = "ANDROID_MUSIC";
const CLIENT_VERSION: &str = "8.47.54";
const USER_AGENT: &str =
    "com.google.android.apps.youtube.music/8.47.54 (Linux; U; Android 14 gzip)";

const INNERTUBE_API: &str = "https://music.youtube.com";

pub struct MusicAndroidClient {
    http: reqwest::Client,
}

impl MusicAndroidClient {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("Failed to build Music Android HTTP client");

        Self { http }
    }

    fn build_context(&self) -> Value {
        json!({
            "client": {
                "clientName": CLIENT_NAME,
                "clientVersion": CLIENT_VERSION,
                "userAgent": USER_AGENT,
                "deviceMake": "Google",
                "deviceModel": "Pixel 6",
                "osName": "Android",
                "osVersion": "14",
                "androidSdkVersion": "30",
                "hl": "en",
                "gl": "US"
            },
            "user": { "lockedSafetyMode": false },
            "request": { "useSsl": true }
        })
    }

    async fn player_request(
        &self,
        video_id: &str,
        oauth: &Arc<YouTubeOAuth>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let body = json!({
            "context": self.build_context(),
            "videoId": video_id,
            "contentCheckOk": true,
            "racyCheckOk": true
        });

        let url = format!("{}/youtubei/v1/player?prettyPrint=false", INNERTUBE_API);

        let mut req = self
            .http
            .post(&url)
            .header("X-YouTube-Client-Name", "67") // 67 is for ANDROID_MUSIC
            .header("X-YouTube-Client-Version", CLIENT_VERSION)
            .header("Origin", INNERTUBE_API)
            .json(&body);

        if let Some(auth) = oauth.get_auth_header().await {
            req = req.header("Authorization", auth);
        }

        let res = req.send().await?;
        let status = res.status();
        if !status.is_success() {
            return Err(format!("Music Android player request returned {status}").into());
        }

        Ok(res.json().await?)
    }
}

#[async_trait]
impl YouTubeClient for MusicAndroidClient {
    fn name(&self) -> &str {
        "MusicAndroid"
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
        _context: &Value,
        oauth: Arc<YouTubeOAuth>,
    ) -> Result<Vec<Track>, Box<dyn std::error::Error + Send + Sync>> {
        let body = json!({
            "context": self.build_context(),
            "query": query,
            "params": "EgWKAQIIAWoQEAMQBBAJEAoQBRAREBAQFQ%3D%3D" // NodeLink Track params
        });

        let url = format!("{}/youtubei/v1/search?prettyPrint=false", INNERTUBE_API);

        let mut req = self
            .http
            .post(&url)
            .header("X-YouTube-Client-Name", "67")
            .header("X-YouTube-Client-Version", CLIENT_VERSION)
            .header("X-Goog-Api-Format-Version", "2")
            .header("Origin", INNERTUBE_API)
            .json(&body);

        if let Some(auth) = oauth.get_auth_header().await {
            req = req.header("Authorization", auth);
        }

        let res = req.send().await?;
        if !res.status().is_success() {
            return Err(format!("Music Android search failed: {}", res.status()).into());
        }

        let response: Value = res.json().await?;
        let mut tracks = Vec::new();

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

                        let mut author = "Unknown Artist".to_string();
                        let mut length_ms = 0u64;

                        if let Some(flex_cols) =
                            renderer.get("flexColumns").and_then(|c| c.as_array())
                        {
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
        oauth: Arc<YouTubeOAuth>,
    ) -> Result<Option<Track>, Box<dyn std::error::Error + Send + Sync>> {
        let body = self.player_request(track_id, &oauth).await?;

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
        oauth: Arc<YouTubeOAuth>,
    ) -> Result<Option<(Vec<Track>, String)>, Box<dyn std::error::Error + Send + Sync>> {
        let body = json!({
            "context": self.build_context(),
            "playlistId": playlist_id,
            "enablePersistentPlaylistPanel": true,
            "isAudioOnly": true
        });

        let url = format!("{}/youtubei/v1/next?prettyPrint=false", INNERTUBE_API);

        let mut req = self
            .http
            .post(&url)
            .header("X-YouTube-Client-Name", "67")
            .header("X-YouTube-Client-Version", CLIENT_VERSION)
            .json(&body);

        if let Some(auth) = oauth.get_auth_header().await {
            req = req.header("Authorization", auth);
        }

        let res = req.send().await?;
        if !res.status().is_success() {
            return Ok(None);
        }

        let body: Value = res.json().await?;
        let result = crate::sources::youtube::extractor::extract_from_next(&body, "youtube");

        if result.is_none() {
            tracing::warn!("MusicAndroid: extract_from_next returned None");
            if let Some(obj) = body.as_object() {
                tracing::warn!("Response keys: {:?}", obj.keys());
            }
            if let Some(contents) = body.get("contents") {
                if let Some(obj) = contents.as_object() {
                    tracing::warn!("Contents keys: {:?}", obj.keys());
                    if let Some(renderer) = obj.get("singleColumnMusicWatchNextResultsRenderer") {
                        tracing::warn!(
                            "Renderer keys: {:?}",
                            renderer.as_object().map(|o| o.keys())
                        );
                    }
                }
            } else {
                tracing::warn!("No 'contents' field in response");
            }
        }

        Ok(result)
    }

    async fn resolve_url(
        &self,
        _url: &str,
        _context: &Value,
        _oauth: Arc<YouTubeOAuth>,
    ) -> Result<Option<Track>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(None)
    }

    async fn get_track_url(
        &self,
        track_id: &str,
        _context: &Value,
        cipher_manager: Arc<YouTubeCipherManager>,
        oauth: Arc<YouTubeOAuth>,
    ) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let body = self.player_request(track_id, &oauth).await?;

        let playability = body
            .get("playabilityStatus")
            .and_then(|p| p.get("status"))
            .and_then(|s| s.as_str())
            .unwrap_or("UNKNOWN");
        if playability != "OK" {
            return Ok(None);
        }

        let streaming_data = match body.get("streamingData") {
            Some(sd) => sd,
            None => return Ok(None),
        };

        if let Some(server_abr_url) = streaming_data
            .get("serverAbrStreamingUrl")
            .and_then(|v| v.as_str())
        {
            let ustreamer_config = body
                .get("playerConfig")
                .and_then(|p| p.get("mediaCommonConfig"))
                .and_then(|m| m.get("mediaUstreamerRequestConfig"))
                .and_then(|m| m.get("videoPlaybackUstreamerConfig"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let visitor_data = body
                .get("responseContext")
                .and_then(|r| r.get("visitorData"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let formats: Vec<Value> = streaming_data.get("adaptiveFormats").and_then(|f| f.as_array()).into_iter().flatten().map(|f| {
                json!({
                    "itag": f.get("itag").and_then(|v| v.as_i64()).unwrap_or(0),
                    "lastModified": f.get("lastModified").or_else(|| f.get("last_modified_ms")).and_then(|v| v.as_str()).and_then(|s| s.parse::<i64>().ok()),
                    "xtags": f.get("xtags").and_then(|v| v.as_str()),
                    "mimeType": f.get("mimeType").and_then(|v| v.as_str()),
                    "bitrate": f.get("bitrate").and_then(|v| v.as_i64()),
                })
            }).collect();

            let sabr_payload = json!({
                "url": server_abr_url,
                "config": ustreamer_config,
                "clientName":      67, // ANDROID_MUSIC
                "clientVersion":   CLIENT_VERSION,
                "visitorData":     visitor_data,
                "formats":         formats,
            });

            let encoded = BASE64_STANDARD.encode(serde_json::to_string(&sabr_payload)?);
            return Ok(Some(format!("sabr://{}", encoded)));
        }

        if let Some(hls) = streaming_data
            .get("hlsManifestUrl")
            .and_then(|v| v.as_str())
        {
            return Ok(Some(hls.to_string()));
        }

        let adaptive = streaming_data
            .get("adaptiveFormats")
            .and_then(|v| v.as_array());
        let formats = streaming_data.get("formats").and_then(|v| v.as_array());
        let player_page_url = format!("https://music.youtube.com/watch?v={}", track_id);

        if let Some(best) = select_best_audio_format(adaptive, formats) {
            match resolve_format_url(best, &player_page_url, &cipher_manager).await {
                Ok(Some(url)) => return Ok(Some(url)),
                Ok(None) => {}
                Err(e) => return Err(e),
            }
        }

        Ok(None)
    }
}
