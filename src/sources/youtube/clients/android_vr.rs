use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use super::{
    YouTubeClient,
    common::{INNERTUBE_API, resolve_format_url, select_best_audio_format},
};
use crate::{
    common::types::AnyResult,
    protocol::tracks::Track,
    sources::youtube::{
        cipher::YouTubeCipherManager, clients::common::ClientConfig, extractor::extract_track,
        oauth::YouTubeOAuth,
    },
};

const CLIENT_NAME: &str = "ANDROID_VR";
const CLIENT_ID: &str = "28";
const CLIENT_VERSION: &str = "1.71.26";
const USER_AGENT: &str = "com.google.android.apps.youtube.vr.oculus/1.71.26 (Linux; U; Android 15; eureka-user Build/AP4A.250205.002) gzip";

pub struct AndroidVrClient {
    http: Arc<reqwest::Client>,
}

impl AndroidVrClient {
    pub fn new(http: Arc<reqwest::Client>) -> Self {
        Self { http }
    }

    fn config(&self) -> ClientConfig<'_> {
        ClientConfig {
            client_name: CLIENT_NAME,
            client_version: CLIENT_VERSION,
            client_id: CLIENT_ID,
            user_agent: USER_AGENT,
            device_make: Some("Google"),
            os_name: Some("Android"),
            os_version: Some("15"),
            android_sdk_version: Some("35"),
            ..Default::default()
        }
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
            &self.config(),
            video_id,
            None,
            visitor_data,
            signature_timestamp,
            None,
            None,
            None,
            None,
        )
        .await
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
        context: &Value,
        _oauth: Arc<YouTubeOAuth>,
    ) -> AnyResult<Vec<Track>> {
        let visitor_data = context
            .get("client")
            .and_then(|c| c.get("visitorData"))
            .and_then(|v| v.as_str())
            .or_else(|| context.get("visitorData").and_then(|v| v.as_str()));

        let body = json!({
            "context": self.config().build_context(visitor_data),
            "query": query,
            "params": "EgIQAQ%3D%3D"
        });

        let url = format!("{}/youtubei/v1/search?prettyPrint=false", INNERTUBE_API);

        let mut req = self
            .http
            .post(&url)
            .header("User-Agent", USER_AGENT)
            .header("X-YouTube-Client-Name", CLIENT_ID)
            .header("X-YouTube-Client-Version", CLIENT_VERSION)
            .header("X-Goog-Api-Format-Version", "2");

        if let Some(vd) = visitor_data {
            req = req.header("X-Goog-Visitor-Id", vd);
        }

        let req = req.json(&body);

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
        _context: &Value,
        _oauth: Arc<YouTubeOAuth>,
    ) -> AnyResult<Option<Track>> {
        tracing::debug!("{} client does not support get_track_info", self.name());
        Ok(None)
    }

    async fn get_playlist(
        &self,
        _playlist_id: &str,
        _context: &Value,
        _oauth: Arc<YouTubeOAuth>,
    ) -> AnyResult<Option<(Vec<Track>, String)>> {
        tracing::debug!("{} client does not support get_playlist", self.name());
        Ok(None)
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

        let selected = select_best_audio_format(adaptive, formats);
        if selected.is_none() {
            tracing::warn!("AndroidVR: no suitable audio format found for {}", track_id);
            return Ok(None);
        }
        let best = selected.unwrap();
        let itag = best.get("itag").and_then(|v| v.as_i64()).unwrap_or(-1);
        let mime = best.get("mimeType").and_then(|v| v.as_str()).unwrap_or("?");
        tracing::debug!(
            "AndroidVR: selected format itag={} mime={} for {}",
            itag,
            mime,
            track_id
        );

        match resolve_format_url(best, &player_page_url, &cipher_manager).await {
            Ok(Some(url)) => {
                tracing::debug!("AndroidVR: resolved URL for {} (itag={})", track_id, itag);
                Ok(Some(url))
            }
            Ok(None) => {
                tracing::warn!(
                    "AndroidVR: resolve_format_url returned None for {} (itag={})",
                    track_id,
                    itag
                );
                Ok(None)
            }
            Err(e) => {
                tracing::warn!(
                    "AndroidVR: resolve_format_url error for {} (itag={}): {}",
                    track_id,
                    itag,
                    e
                );
                Ok(None)
            }
        }
    }

    async fn get_streaming_data(
        &self,
        track_id: &str,
        context: &Value,
        _cipher_manager: Arc<YouTubeCipherManager>,
        oauth: Arc<YouTubeOAuth>,
    ) -> crate::common::types::AnyResult<Option<Value>> {
        let visitor_data = context.get("visitorData").and_then(|v| v.as_str());
        let body = self.player_request(track_id, visitor_data, None, &oauth).await?;
        if body.get("playabilityStatus").and_then(|p| p.get("status")).and_then(|s| s.as_str()) != Some("OK") {
            return Ok(None);
        }
        Ok(body.get("streamingData").cloned())
    }
}
