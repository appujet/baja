use super::YouTubeClient;
use super::common::{INNERTUBE_API, resolve_format_url, select_best_audio_format};
use crate::api::tracks::Track;
use crate::sources::youtube::cipher::YouTubeCipherManager;
use crate::sources::youtube::extractor::extract_track;
use crate::sources::youtube::oauth::YouTubeOAuth;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::Arc;


const CLIENT_NAME: &str = "ANDROID_VR";
const CLIENT_ID: &str = "28";
const CLIENT_VERSION: &str = "1.61.48";
const USER_AGENT: &str = "Mozilla/5.0 (Linux; Android 14; Pixel 8 Pro Build/UQ1A.240205.002; wv) \
     AppleWebKit/537.36 (KHTML, like Gecko) Version/4.0 \
     Chrome/121.0.6167.164 Mobile Safari/537.36 YouTubeVR/1.61.48 (gzip)";

pub struct AndroidVrClient {
    http: reqwest::Client,
}

impl AndroidVrClient {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("Failed to build AndroidVR HTTP client");

        Self { http }
    }

    fn build_context(&self) -> Value {
        json!({
            "client": {
                "clientName": CLIENT_NAME,
                "clientVersion": CLIENT_VERSION,
                "userAgent": USER_AGENT,
                "androidSdkVersion": 34,
                "deviceMake": "Google",
                "deviceModel": "Pixel 8 Pro",
                "osName": "Android",
                "osVersion": "14",
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
            return Err(format!("AndroidVR player request returned {status}").into());
        }

        Ok(res.json().await?)
    }
}


#[async_trait]
impl YouTubeClient for AndroidVrClient {
    fn name(&self) -> &str {
        "AndroidVR"
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
            "params": "EgIQAQ%3D%3D"
        });

        let url = format!("{}/youtubei/v1/search?prettyPrint=false", INNERTUBE_API);

        let mut req = self
            .http
            .post(&url)
            .header("X-YouTube-Client-Name", CLIENT_ID)
            .header("X-YouTube-Client-Version", CLIENT_VERSION)
            .json(&body);

        if let Some(auth) = oauth.get_auth_header().await {
            req = req.header("Authorization", auth);
        }

        let res = req.send().await?;
        if !res.status().is_success() {
            return Err(format!("AndroidVR search failed: {}", res.status()).into());
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
                "AndroidVR player: video {} not playable (status={})",
                track_id,
                playability
            );
            return Ok(None);
        }

        let streaming_data = match body.get("streamingData") {
            Some(sd) => sd,
            None => {
                tracing::error!("AndroidVR player: no streamingData for {}", track_id);
                return Ok(None);
            }
        };

        // HLS for live content
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
        let player_page_url = format!("https://www.youtube.com/watch?v={}", track_id);

        if let Some(best) = select_best_audio_format(adaptive, formats) {
            if let Ok(Some(url)) = resolve_format_url(best, &player_page_url, &cipher_manager).await
            {
                return Ok(Some(url));
            }
        }

        Ok(None)
    }
}
