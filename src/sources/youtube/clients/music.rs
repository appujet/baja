use super::YouTubeClient;
use super::common::{resolve_format_url, select_best_audio_format};
use crate::api::tracks::{Track, TrackInfo};
use crate::sources::youtube::cipher::YouTubeCipherManager;
use crate::sources::youtube::oauth::YouTubeOAuth;
use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use serde_json::{Value, json};
use std::sync::Arc;

// ─── Constants ────────────────────────────────────────────────────────────────

const CLIENT_NAME: &str = "ANDROID_MUSIC";
/// InnerTube X-YouTube-Client-Name ID for ANDROID_MUSIC.
const CLIENT_ID: &str = "21";
const CLIENT_VERSION: &str = "7.27.52";
const USER_AGENT: &str =
    "com.google.android.apps.youtube.music/7.27.52 (Linux; U; Android 14) gzip";

/// YouTube Music uses its own domain for the InnerTube API.
const MUSIC_API: &str = "https://music.youtube.com";

// ─── Client ───────────────────────────────────────────────────────────────────

pub struct MusicClient {
    http: reqwest::Client,
}

impl MusicClient {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("Failed to build Music HTTP client");

        Self { http }
    }

    fn build_context(&self) -> Value {
        json!({
            "client": {
                "clientName": CLIENT_NAME,
                "clientVersion": CLIENT_VERSION,
                "userAgent": USER_AGENT,
                "androidSdkVersion": 34,
                "osName": "Android",
                "osVersion": "14",
                "deviceMake": "Google",
                "deviceModel": "Pixel 6",
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

        let url = format!("{}/youtubei/v1/player?prettyPrint=false", MUSIC_API);

        let mut req = self
            .http
            .post(&url)
            .header("X-YouTube-Client-Name", CLIENT_ID)
            .header("X-YouTube-Client-Version", CLIENT_VERSION)
            .header("Origin", MUSIC_API)
            .json(&body);

        if let Some(auth) = oauth.get_auth_header().await {
            req = req.header("Authorization", auth);
        }

        let res = req.send().await?;
        let status = res.status();
        if !status.is_success() {
            return Err(format!("Music player request returned {status}").into());
        }

        Ok(res.json().await?)
    }
}

// ─── Trait impl ───────────────────────────────────────────────────────────────

#[async_trait]
impl YouTubeClient for MusicClient {
    fn name(&self) -> &str {
        "Music"
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
        // YouTube Music uses a dedicated search endpoint with a music-specific param.
        let body = json!({
            "context": self.build_context(),
            "query": query,
            "params": "EgWKAQIIAWoQEAMQBBAJEAoQBRAREBAQFQ%3D%3D"
        });

        let url = format!("{}/youtubei/v1/search?prettyPrint=false", MUSIC_API);

        let mut req = self
            .http
            .post(&url)
            .header("X-YouTube-Client-Name", CLIENT_ID)
            .header("X-YouTube-Client-Version", CLIENT_VERSION)
            .header("X-Goog-Api-Format-Version", "2")
            .header("Origin", MUSIC_API)
            .json(&body);

        if let Some(auth) = oauth.get_auth_header().await {
            req = req.header("Authorization", auth);
        }

        let res = req.send().await?;
        if !res.status().is_success() {
            return Err(format!("Music search failed: {}", res.status()).into());
        }

        let response: Value = res.json().await?;
        let mut tracks = Vec::new();

        // Navigate: contents → tabbedSearchResultsRenderer → tabs[0] → tabRenderer → content
        //           → sectionListRenderer → contents → [musicShelfRenderer] → contents
        let shelf_items = response
            .get("contents")
            .and_then(|c| c.get("tabbedSearchResultsRenderer"))
            .and_then(|t| t.get("tabs"))
            .and_then(|t| t.get(0))
            .and_then(|t| t.get("tabRenderer"))
            .and_then(|t| t.get("content"))
            .and_then(|c| c.get("sectionListRenderer"))
            .and_then(|s| s.get("contents"))
            .and_then(|c| c.as_array())
            .and_then(|sections| {
                sections.iter().find_map(|s| {
                    s.get("musicShelfRenderer")
                        .and_then(|m| m.get("contents"))
                        .and_then(|c| c.as_array())
                })
            });

        if let Some(items) = shelf_items {
            for item in items {
                let renderer = item
                    .get("musicResponsiveListItemRenderer")
                    .or_else(|| item.get("musicTwoColumnItemRenderer"));

                if let Some(renderer) = renderer {
                    // Extract video ID from playlistItemData or doubleTapCommand
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
                        });

                    // Extract title from flexColumns
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

                    // Extract artist from second flex column runs
                    let author = renderer
                        .get("flexColumns")
                        .and_then(|c| c.get(1))
                        .and_then(|c| c.get("musicResponsiveListItemFlexColumnRenderer"))
                        .and_then(|r| r.get("text"))
                        .and_then(|t| t.get("runs"))
                        .and_then(|r| r.get(0))
                        .and_then(|r| r.get("text"))
                        .and_then(|t| t.as_str())
                        .unwrap_or("Unknown Artist");

                    if let Some(id) = id {
                        let info = TrackInfo {
                            identifier: id.to_string(),
                            is_seekable: true,
                            title: title.to_string(),
                            author: author.to_string(),
                            length: 0,
                            is_stream: false,
                            uri: Some(format!("https://music.youtube.com/watch?v={}", id)),
                            source_name: "youtube".to_string(),
                            isrc: None,
                            artwork_url: None,
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
        _track_id: &str,
        _oauth: Arc<YouTubeOAuth>,
    ) -> Result<Option<Track>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(None)
    }

    async fn get_playlist(
        &self,
        _playlist_id: &str,
        _oauth: Arc<YouTubeOAuth>,
    ) -> Result<Option<(Vec<Track>, String)>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(None)
    }

    async fn resolve_url(
        &self,
        _url: &str,
        _context: &Value,
    ) -> Result<Option<Track>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(None)
    }

    async fn get_track_url(
        &self,
        track_id: &str,
        _context: &Value,
        cipher_manager: Arc<YouTubeCipherManager>,
    ) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let oauth = Arc::new(YouTubeOAuth::new(vec![]));
        let body = self.player_request(track_id, &oauth).await?;

        let playability = body
            .get("playabilityStatus")
            .and_then(|p| p.get("status"))
            .and_then(|s| s.as_str())
            .unwrap_or("UNKNOWN");

        if playability != "OK" {
            let reason = body
                .get("playabilityStatus")
                .and_then(|p| p.get("reason"))
                .and_then(|r| r.as_str())
                .unwrap_or("unknown reason");
            tracing::warn!(
                "Music player: video {} not playable (status={}, reason={})",
                track_id,
                playability,
                reason
            );
            return Ok(None);
        }

        let streaming_data = match body.get("streamingData") {
            Some(sd) => sd,
            None => {
                tracing::error!("Music player: no streamingData for {}", track_id);
                return Ok(None);
            }
        };

        // ── SABR path ────────────────────────────────────────────────────────
        if let Some(server_abr_url) = streaming_data
            .get("serverAbrStreamingUrl")
            .and_then(|v| v.as_str())
        {
            tracing::debug!("Music player: SABR URL found for {}", track_id);

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

            let formats: Vec<Value> = streaming_data
                .get("adaptiveFormats")
                .and_then(|f| f.as_array())
                .into_iter()
                .flatten()
                .map(|f| {
                    json!({
                        "itag": f.get("itag").and_then(|v| v.as_i64()).unwrap_or(0),
                        "lastModified": f.get("lastModified")
                            .or_else(|| f.get("last_modified_ms"))
                            .and_then(|v| v.as_str())
                            .and_then(|s| s.parse::<i64>().ok()),
                        "xtags": f.get("xtags").and_then(|v| v.as_str()),
                        "mimeType": f.get("mimeType").and_then(|v| v.as_str()),
                        "bitrate": f.get("bitrate").and_then(|v| v.as_i64()),
                    })
                })
                .collect();

            let sabr_payload = json!({
                "url":           server_abr_url,
                "config":        ustreamer_config,
                "clientName":    21,  // ANDROID_MUSIC
                "clientVersion": CLIENT_VERSION,
                "visitorData":   visitor_data,
                "formats":       formats,
            });

            let encoded = BASE64_STANDARD.encode(serde_json::to_string(&sabr_payload)?);
            return Ok(Some(format!("sabr://{}", encoded)));
        }

        // ── HLS path (live streams) ───────────────────────────────────────────
        if let Some(hls) = streaming_data
            .get("hlsManifestUrl")
            .and_then(|v| v.as_str())
        {
            tracing::debug!("Music player: using HLS manifest for {}", track_id);
            return Ok(Some(hls.to_string()));
        }

        // ── Direct / ciphered format path ────────────────────────────────────
        let adaptive = streaming_data
            .get("adaptiveFormats")
            .and_then(|v| v.as_array());
        let formats = streaming_data.get("formats").and_then(|v| v.as_array());
        let player_page_url = format!("https://music.youtube.com/watch?v={}", track_id);

        if let Some(best) = select_best_audio_format(adaptive, formats) {
            match resolve_format_url(best, &player_page_url, &cipher_manager).await {
                Ok(Some(url)) => {
                    tracing::debug!(
                        "Music player: resolved audio URL for {} (itag={})",
                        track_id,
                        best.get("itag").and_then(|v| v.as_i64()).unwrap_or(-1)
                    );
                    return Ok(Some(url));
                }
                Ok(None) => {
                    tracing::warn!(
                        "Music player: best format had no resolvable URL for {}",
                        track_id
                    );
                }
                Err(e) => {
                    tracing::error!(
                        "Music player: cipher resolution failed for {}: {}",
                        track_id,
                        e
                    );
                    return Err(e);
                }
            }
        }

        tracing::warn!(
            "Music player: no suitable audio format found for {}",
            track_id
        );
        Ok(None)
    }
}
