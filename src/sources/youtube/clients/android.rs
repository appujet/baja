use super::YouTubeClient;
use super::common::{INNERTUBE_API, resolve_format_url, select_best_audio_format};
use crate::api::tracks::Track;
use crate::sources::youtube::cipher::YouTubeCipherManager;
use crate::sources::youtube::extractor::{extract_from_player, extract_track};
use crate::sources::youtube::oauth::YouTubeOAuth;
use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use serde_json::{Value, json};
use std::sync::Arc;

const CLIENT_NAME: &str = "ANDROID";
const CLIENT_ID: &str = "3";
const CLIENT_VERSION: &str = "20.01.35";
const USER_AGENT: &str = "com.google.android.youtube/20.01.35 (Linux; U; Android 14) identity";

pub struct AndroidClient {
    http: reqwest::Client,
}

impl AndroidClient {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("Failed to build Android HTTP client");

        Self { http }
    }

    /// Build the InnerTube context for Android.
    /// Mirrors NodeLink's Android.js `getClient()` — passes visitorData when available.
    fn build_context(&self, visitor_data: Option<&str>) -> Value {
        let mut client = json!({
            "clientName": CLIENT_NAME,
            "clientVersion": CLIENT_VERSION,
            "userAgent": USER_AGENT,
            "deviceMake": "Google",
            "deviceModel": "Pixel 6",
            "osName": "Android",
            "osVersion": "14",
            "androidSdkVersion": "34",
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
        _oauth: &Arc<YouTubeOAuth>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let body = json!({
            "context": self.build_context(visitor_data),
            "videoId": video_id,
            "contentCheckOk": true,
            "racyCheckOk": true
        });

        let url = format!("{}/youtubei/v1/player?prettyPrint=false", INNERTUBE_API);

        let mut req = self
            .http
            .post(&url)
            .header("X-YouTube-Client-Name", CLIENT_ID)
            .header("X-YouTube-Client-Version", CLIENT_VERSION);

        if let Some(vd) = visitor_data {
            req = req.header("X-Goog-Visitor-Id", vd);
        }

        let req = req.json(&body);

        tracing::debug!("Android player request URL: {}", url);
        tracing::debug!(
            "Android player request body: {}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );

        let res = req.send().await?;
        let status = res.status();
        let body_text = res.text().await?;

        tracing::debug!("Android player response status: {}", status);
        tracing::debug!("Android player response body: {}", body_text);

        if !status.is_success() {
            return Err(format!("Android player request returned {status}: {body_text}").into());
        }

        Ok(serde_json::from_str(&body_text)?)
    }
}

#[async_trait]
impl YouTubeClient for AndroidClient {
    fn name(&self) -> &str {
        "Android"
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
        _oauth: Arc<YouTubeOAuth>,
    ) -> Result<Vec<Track>, Box<dyn std::error::Error + Send + Sync>> {
        let visitor_data = context
            .get("client")
            .and_then(|c| c.get("visitorData"))
            .and_then(|v| v.as_str())
            .or_else(|| context.get("visitorData").and_then(|v| v.as_str()));

        let body = json!({
            "context": self.build_context(visitor_data),
            "query": query,
            "params": "EgIQAQ%3D%3D"
        });

        let url = format!("{}/youtubei/v1/search", INNERTUBE_API);

        let req = self
            .http
            .post(&url)
            .header("X-Goog-Api-Format-Version", "2")
            .header("X-Goog-Visitor-Id", visitor_data.unwrap_or(""))
            .header("X-YouTube-Client-Name", CLIENT_ID)
            .header("X-YouTube-Client-Version", CLIENT_VERSION);

        let req = req.json(&body);

        let res = req.send().await?;
        let status = res.status();
        let body_text = res.text().await?;

        if !status.is_success() {
            return Err(format!("Android search failed: {} - {}", status, body_text).into());
        }

        let response: Value = serde_json::from_str(&body_text).unwrap_or_default();
        let mut tracks = Vec::new();

        if let Some(sections) = response
            .get("contents")
            .and_then(|c| c.get("sectionListRenderer"))
            .and_then(|s| s.get("contents"))
            .and_then(|c| c.as_array())
        {
            for section in sections {
                // Try itemSectionRenderer first
                let items_opt = section
                    .get("itemSectionRenderer")
                    .and_then(|i| i.get("contents"))
                    .and_then(|c| c.as_array());

                // Also try shelfRenderer / richShelfRenderer
                let shelf_items_opt = items_opt
                    .is_none()
                    .then(|| {
                        let shelf = section
                            .get("shelfRenderer")
                            .or_else(|| section.get("richShelfRenderer"));
                        shelf.and_then(|s| {
                            s.get("content")
                                .and_then(|c| c.get("verticalListRenderer"))
                                .and_then(|v| v.get("items"))
                                .or_else(|| {
                                    s.get("content")
                                        .and_then(|c| c.get("richGridRenderer"))
                                        .and_then(|r| r.get("contents"))
                                })
                                .and_then(|c| c.as_array())
                        })
                    })
                    .flatten();

                let items = items_opt.or(shelf_items_opt);

                if let Some(items) = items {
                    for item in items {
                        // Unwrap richItemRenderer wrapper if present
                        let inner = item
                            .get("richItemRenderer")
                            .and_then(|r| r.get("content"))
                            .unwrap_or(item);

                        if let Some(track) = extract_track(inner, "youtube") {
                            tracks.push(track);
                        }
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
    ) -> Result<Option<Track>, Box<dyn std::error::Error + Send + Sync>> {
        let visitor_data = context
            .get("client")
            .and_then(|c| c.get("visitorData"))
            .and_then(|v| v.as_str())
            .or_else(|| context.get("visitorData").and_then(|v| v.as_str()));

        let body = self.player_request(track_id, visitor_data, &oauth).await?;
        Ok(extract_from_player(&body, "youtube"))
    }

    async fn get_playlist(
        &self,
        playlist_id: &str,
        context: &Value,
        oauth: Arc<YouTubeOAuth>,
    ) -> Result<Option<(Vec<Track>, String)>, Box<dyn std::error::Error + Send + Sync>> {
        let visitor_data = context
            .get("client")
            .and_then(|c| c.get("visitorData"))
            .and_then(|v| v.as_str())
            .or_else(|| context.get("visitorData").and_then(|v| v.as_str()));

        let body = json!({
            "context": self.build_context(visitor_data),
            "playlistId": playlist_id,
            "contentCheckOk": true,
            "racyCheckOk": true
        });

        let url = format!("{}/youtubei/v1/next?prettyPrint=false", INNERTUBE_API);

        let mut req = self
            .http
            .post(&url)
            .header("X-YouTube-Client-Name", CLIENT_ID)
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
    ) -> Result<Option<Track>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(None)
    }

    async fn get_track_url(
        &self,
        track_id: &str,
        context: &Value,
        cipher_manager: Arc<YouTubeCipherManager>,
        oauth: Arc<YouTubeOAuth>,
    ) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let visitor_data = context
            .get("client")
            .and_then(|c| c.get("visitorData"))
            .and_then(|v| v.as_str())
            .or_else(|| context.get("visitorData").and_then(|v| v.as_str()));

        let body = self.player_request(track_id, visitor_data, &oauth).await?;

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
                "Android player: video {} not playable (status={}, reason={})",
                track_id,
                playability,
                reason
            );
            return Ok(None);
        }

        let streaming_data = match body.get("streamingData") {
            Some(sd) => sd,
            None => {
                tracing::error!("Android player: no streamingData for {}", track_id);
                return Ok(None);
            }
        };

        if let Some(server_abr_url) = streaming_data
            .get("serverAbrStreamingUrl")
            .and_then(|v| v.as_str())
        {
            tracing::debug!("Android player: SABR URL found for {}", track_id);

            let ustreamer_config = body
                .get("playerConfig")
                .and_then(|p| p.get("mediaCommonConfig"))
                .and_then(|m| m.get("mediaUstreamerRequestConfig"))
                .and_then(|m| m.get("videoPlaybackUstreamerConfig"))
                .and_then(|v| v.as_str())
                .unwrap_or("");

            // Use visitorData from the response itself
            let response_visitor_data = body
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
                        "itag":         f.get("itag").and_then(|v| v.as_i64()).unwrap_or(0),
                        "lastModified": f.get("lastModified")
                            .or_else(|| f.get("last_modified_ms"))
                            .and_then(|v| v.as_str())
                            .and_then(|s| s.parse::<i64>().ok()),
                        "xtags":        f.get("xtags").and_then(|v| v.as_str()),
                        "mimeType":     f.get("mimeType").and_then(|v| v.as_str()),
                        "bitrate":      f.get("bitrate").and_then(|v| v.as_i64()),
                        "audioQuality": f.get("audioQuality").and_then(|v| v.as_str()),
                    })
                })
                .collect();

            let sabr_payload = json!({
                "url":           server_abr_url,
                "config":        ustreamer_config,
                "clientName":    3,  // ANDROID
                "clientVersion": CLIENT_VERSION,
                "visitorData":   response_visitor_data,
                "formats":       formats,
            });

            let encoded = BASE64_STANDARD.encode(serde_json::to_string(&sabr_payload)?);
            return Ok(Some(format!("sabr://{}", encoded)));
        }

        if let Some(hls) = streaming_data
            .get("hlsManifestUrl")
            .and_then(|v| v.as_str())
        {
            tracing::debug!("Android player: using HLS manifest for {}", track_id);
            return Ok(Some(hls.to_string()));
        }

        let adaptive = streaming_data
            .get("adaptiveFormats")
            .and_then(|v| v.as_array());
        let formats = streaming_data.get("formats").and_then(|v| v.as_array());

        let player_page_url = format!("https://www.youtube.com/watch?v={}", track_id);

        if let Some(best) = select_best_audio_format(adaptive, formats) {
            match resolve_format_url(best, &player_page_url, &cipher_manager).await {
                Ok(Some(url)) => {
                    tracing::debug!(
                        "Android player: resolved audio URL for {} (itag={})",
                        track_id,
                        best.get("itag").and_then(|v| v.as_i64()).unwrap_or(-1)
                    );
                    return Ok(Some(url));
                }
                Ok(None) => {
                    tracing::warn!(
                        "Android player: best format had no resolvable URL for {}",
                        track_id
                    );
                }
                Err(e) => {
                    tracing::error!(
                        "Android player: cipher resolution failed for {}: {}",
                        track_id,
                        e
                    );
                    return Err(e);
                }
            }
        }

        tracing::warn!(
            "Android player: no suitable audio format found for {}",
            track_id
        );
        Ok(None)
    }
}
