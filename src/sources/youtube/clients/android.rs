
use crate::common::types::{AnyResult};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use super::{
    YouTubeClient,
    common::{INNERTUBE_API, resolve_format_url, select_best_audio_format},
};
use crate::{
    api::tracks::Track,
    sources::youtube::{
        cipher::YouTubeCipherManager,
        extractor::{extract_from_player, extract_track},
        oauth::YouTubeOAuth,
    },
};

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
        signature_timestamp: Option<u32>,
        _oauth: &Arc<YouTubeOAuth>,
    ) -> AnyResult<Value> {
        crate::sources::youtube::clients::common::make_player_request(
            &self.http,
            video_id,
            self.build_context(visitor_data),
            CLIENT_ID,
            CLIENT_VERSION,
            None,
            visitor_data,
            signature_timestamp,
            None,
            None,
            None,
        )
        .await
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
    ) -> AnyResult<Vec<Track>> {
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
    ) -> AnyResult<Option<Track>> {
        let visitor_data = context
            .get("client")
            .and_then(|c| c.get("visitorData"))
            .and_then(|v| v.as_str())
            .or_else(|| context.get("visitorData").and_then(|v| v.as_str()));

        let body = self
            .player_request(track_id, visitor_data, None, &oauth)
            .await?;
        Ok(extract_from_player(&body, "youtube"))
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
    ) -> AnyResult<Option<Track>> {
        Ok(None)
    }

    async fn get_track_url(
        &self,
        track_id: &str,
        context: &Value,
        cipher_manager: Arc<YouTubeCipherManager>,
        oauth: Arc<YouTubeOAuth>,
    ) -> AnyResult<Option<String>> {
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

        if let Some(sabr_url) = crate::sources::youtube::clients::common::extract_sabr_payload(
            &body,
            streaming_data,
            3, // ANDROID client_id = 3
            CLIENT_VERSION,
        ) {
            return Ok(Some(sabr_url));
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
