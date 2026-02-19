use super::YouTubeClient;
use super::common::{INNERTUBE_API, resolve_format_url, select_best_audio_format};
use crate::api::tracks::Track;
use crate::sources::youtube::cipher::YouTubeCipherManager;
use crate::sources::youtube::extractor::{extract_from_player, extract_track};
use crate::sources::youtube::oauth::YouTubeOAuth;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::Arc;

const CLIENT_NAME: &str = "IOS";
const CLIENT_VERSION: &str = "21.02.1";
const USER_AGENT: &str =
    "com.google.ios.youtube/21.02.1 (iPhone16,2; U; CPU iOS 18_2 like Mac OS X;)";

pub struct IosClient {
    http: reqwest::Client,
}

impl IosClient {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("Failed to build IOS HTTP client");

        Self { http }
    }

    /// Build the InnerTube context block mirroring NodeLink's IOS getClient().
    fn build_context(&self) -> Value {
        json!({
            "client": {
                "clientName": CLIENT_NAME,
                "clientVersion": CLIENT_VERSION,
                "userAgent": USER_AGENT,
                "deviceMake": "Apple",
                "deviceModel": "iPhone16,2",
                "osName": "iPhone",
                "osVersion": "18.2.22C152",
                "hl": "en",
                "gl": "US",
                "utcOffsetMinutes": 0
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
            .header("X-YouTube-Client-Name", "5") // IOS client_id = 5
            .header("X-YouTube-Client-Version", CLIENT_VERSION)
            .json(&body);

        if let Some(auth) = oauth.get_auth_header().await {
            req = req.header("Authorization", auth);
        }

        let res = req.send().await?;
        let status = res.status();
        if !status.is_success() {
            return Err(format!("IOS player request failed with status {status}").into());
        }

        Ok(res.json().await?)
    }
}

#[async_trait]
impl YouTubeClient for IosClient {
    fn name(&self) -> &str {
        "IOS"
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

    // IOS client delegates search to Web (same as NodeLink's IOS.js).
    async fn search(
        &self,
        query: &str,
        context: &Value,
        oauth: Arc<YouTubeOAuth>,
    ) -> Result<Vec<Track>, Box<dyn std::error::Error + Send + Sync>> {
        let body = json!({
            "context": self.build_context(),
            "query": query,
            "params": "EgIQAQ%3D%3D"
        });

        let url = format!("{}/youtubei/v1/search?prettyPrint=false", INNERTUBE_API);

        let mut req = self
            .http
            .post(&url)
            .header("X-YouTube-Client-Name", "5")
            .header("X-YouTube-Client-Version", CLIENT_VERSION)
            .header("X-Goog-Api-Format-Version", "2")
            .json(&body);

        if let Some(auth) = oauth.get_auth_header().await {
            req = req.header("Authorization", auth);
        }

        let res = req.send().await?;
        if !res.status().is_success() {
            return Err(format!("IOS search failed: {}", res.status()).into());
        }

        let response: Value = res.json().await?;
        let mut tracks = Vec::new();

        if let Some(sections) = response
            .get("contents")
            .and_then(|c| c.get("sectionListRenderer"))
            .and_then(|s| s.get("contents"))
            .and_then(|c| c.as_array())
        {
            for section in sections {
                if let Some(items) = section
                    .get("itemSectionRenderer")
                    .and_then(|i| i.get("contents"))
                    .and_then(|c| c.as_array())
                {
                    for item in items {
                        if let Some(track) = extract_track(item, "youtube") {
                            tracks.push(track);
                        }
                    }
                }
            }
        }

        let _ = context; // not needed for IOS search
        Ok(tracks)
    }

    async fn get_track_info(
        &self,
        track_id: &str,
        oauth: Arc<YouTubeOAuth>,
    ) -> Result<Option<Track>, Box<dyn std::error::Error + Send + Sync>> {
        let body = self.player_request(track_id, &oauth).await?;
        Ok(extract_from_player(&body, "youtube"))
    }

    // IOS doesn't handle playlists natively (same as NodeLink).
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
        // IOS does NOT require a player script for cipher – URLs come pre-signed.
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
                "IOS player: video {} not playable (status={}, reason={})",
                track_id,
                playability,
                reason
            );
            return Ok(None);
        }

        let streaming_data = match body.get("streamingData") {
            Some(sd) => sd,
            None => {
                tracing::error!("IOS player: no streamingData for {}", track_id);
                return Ok(None);
            }
        };

        // HLS path (for live streams)
        if let Some(hls) = streaming_data
            .get("hlsManifestUrl")
            .and_then(|v| v.as_str())
        {
            tracing::debug!("IOS player: using HLS manifest for {}", track_id);
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
                        "IOS player: resolved audio URL for {} (itag={})",
                        track_id,
                        best.get("itag").and_then(|v| v.as_i64()).unwrap_or(-1)
                    );
                    return Ok(Some(url));
                }
                Ok(None) => {
                    tracing::warn!(
                        "IOS player: best format had no resolvable URL for {}",
                        track_id
                    );
                }
                Err(e) => {
                    tracing::error!(
                        "IOS player: cipher resolution failed for {}: {}",
                        track_id,
                        e
                    );
                    return Err(e);
                }
            }
        }

        tracing::warn!(
            "IOS player: no suitable audio format found for {}",
            track_id
        );
        Ok(None)
    }
}
