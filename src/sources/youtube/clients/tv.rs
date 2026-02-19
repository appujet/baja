use super::YouTubeClient;
use super::common::{INNERTUBE_API, resolve_format_url, select_best_audio_format};
use crate::api::tracks::Track;
use crate::sources::youtube::cipher::YouTubeCipherManager;
use crate::sources::youtube::extractor::extract_track;
use crate::sources::youtube::oauth::YouTubeOAuth;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::Arc;


const CLIENT_NAME: &str = "TVHTML5";
const CLIENT_ID: &str = "7";
const CLIENT_VERSION: &str = "7.20250219.19.00";
const USER_AGENT: &str = "Mozilla/5.0 (SmartHub; SMART-TV; U; Linux/SmartTV; Maple2012) \
     AppleWebKit/534.7 (KHTML, like Gecko) SmartTV Safari/534.7";

pub struct TvClient {
    http: reqwest::Client,
}

impl TvClient {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("Failed to build TV HTTP client");

        Self { http }
    }

    fn build_context(&self) -> Value {
        json!({
            "client": {
                "clientName": CLIENT_NAME,
                "clientVersion": CLIENT_VERSION,
                "userAgent": USER_AGENT,
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
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let body = json!({
            "context": self.build_context(),
            "videoId": video_id,
            "contentCheckOk": true,
            "racyCheckOk": true
        });

        let url = format!("{}/youtubei/v1/player?prettyPrint=false", INNERTUBE_API);

        let res = self
            .http
            .post(&url)
            .header("X-YouTube-Client-Name", CLIENT_ID)
            .header("X-YouTube-Client-Version", CLIENT_VERSION)
            .json(&body)
            .send()
            .await?;

        let status = res.status();
        if !status.is_success() {
            return Err(format!("TV player request returned {status}").into());
        }

        Ok(res.json().await?)
    }
}


#[async_trait]
impl YouTubeClient for TvClient {
    fn name(&self) -> &str {
        "TV"
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
            "query": query
        });

        let url = format!("{}/youtubei/v1/search?prettyPrint=false", INNERTUBE_API);

        let mut req = self
            .http
            .post(&url)
            .header("X-YouTube-Client-Name", CLIENT_ID)
            .header("X-YouTube-Client-Version", CLIENT_VERSION)
            .header("X-Goog-Api-Format-Version", "2")
            .json(&body);

        if let Some(auth) = oauth.get_auth_header().await {
            req = req.header("Authorization", auth);
        }

        let res = req.send().await?;
        if !res.status().is_success() {
            return Err(format!("TV search failed: {}", res.status()).into());
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
        let body = self.player_request(track_id).await?;

        let playability = body
            .get("playabilityStatus")
            .and_then(|p| p.get("status"))
            .and_then(|s| s.as_str())
            .unwrap_or("UNKNOWN");

        if playability != "OK" {
            tracing::warn!(
                "TV player: video {} not playable (status={})",
                track_id,
                playability
            );
            return Ok(None);
        }

        let streaming_data = match body.get("streamingData") {
            Some(sd) => sd,
            None => {
                tracing::error!("TV player: no streamingData for {}", track_id);
                return Ok(None);
            }
        };

        // TV client gets HLS for live streams
        if let Some(hls) = streaming_data
            .get("hlsManifestUrl")
            .and_then(|v| v.as_str())
        {
            tracing::debug!("TV player: using HLS manifest for {}", track_id);
            return Ok(Some(hls.to_string()));
        }

        let adaptive = streaming_data
            .get("adaptiveFormats")
            .and_then(|v| v.as_array());
        let formats = streaming_data.get("formats").and_then(|v| v.as_array());
        let player_page_url = format!("https://www.youtube.com/watch?v={}", track_id);

        if let Some(best) = select_best_audio_format(adaptive, formats) {
            match resolve_format_url(best, &player_page_url, &cipher_manager).await {
                Ok(Some(url)) => return Ok(Some(url)),
                Ok(None) => {
                    tracing::warn!(
                        "TV player: best format had no resolvable URL for {}",
                        track_id
                    );
                }
                Err(e) => {
                    tracing::error!(
                        "TV player: cipher resolution failed for {}: {}",
                        track_id,
                        e
                    );
                    return Err(e);
                }
            }
        }

        Ok(None)
    }
}
