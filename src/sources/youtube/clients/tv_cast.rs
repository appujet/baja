use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use super::{
    YouTubeClient,
    common::{INNERTUBE_API, make_next_request, resolve_format_url, select_best_audio_format},
};
use crate::{
    common::types::AnyResult,
    protocol::tracks::Track,
    sources::youtube::{
        cipher::YouTubeCipherManager,
        clients::common::ClientConfig,
        extractor::{extract_from_next, extract_from_player, extract_track},
        oauth::YouTubeOAuth,
    },
};

const CLIENT_NAME_OVERRIDE: &str = "TVHTML5_CAST";
const CLIENT_VERSION: &str = "7.20190924";
const USER_AGENT: &str = "Mozilla/5.0 (Linux; Android) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/145.0.0.0 Safari/537.36 CrKey/1.54.248666";

pub struct TvCastClient {
    http: Arc<reqwest::Client>,
}

impl TvCastClient {
    pub fn new(http: Arc<reqwest::Client>) -> Self {
        Self { http }
    }

    fn config(&self) -> ClientConfig<'_> {
        ClientConfig {
            client_name: CLIENT_NAME_OVERRIDE,
            client_version: CLIENT_VERSION,
            client_id: "7",
            user_agent: USER_AGENT,
            ..Default::default()
        }
    }

    async fn player_request(
        &self,
        video_id: &str,
        visitor_data: Option<&str>,
        signature_timestamp: Option<u32>,
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

    async fn next_request(
        &self,
        video_id: Option<&str>,
        playlist_id: Option<&str>,
        visitor_data: Option<&str>,
    ) -> AnyResult<Value> {
        make_next_request(
            &self.http,
            &self.config(),
            video_id,
            playlist_id,
            visitor_data,
            None,
        )
        .await
    }
}

#[async_trait]
impl YouTubeClient for TvCastClient {
    fn name(&self) -> &str {
        "TV Cast"
    }
    fn client_name(&self) -> &str {
        CLIENT_NAME_OVERRIDE
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
            "query": query
        });

        let url = format!("{}/youtubei/v1/search?prettyPrint=false", INNERTUBE_API);

        let mut req = self
            .http
            .post(&url)
            .header("User-Agent", USER_AGENT)
            .header("X-YouTube-Client-Name", "7")
            .header("X-YouTube-Client-Version", CLIENT_VERSION)
            .header("X-Goog-Api-Format-Version", "2");

        if let Some(vd) = visitor_data {
            req = req.header("X-Goog-Visitor-Id", vd);
        }

        let req = req.json(&body);

        let res = req.send().await?;
        if !res.status().is_success() {
            return Err(format!("TV Cast search failed: {}", res.status()).into());
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
        track_id: &str,
        context: &Value,
        _oauth: Arc<YouTubeOAuth>,
    ) -> AnyResult<Option<Track>> {
        let visitor_data = context
            .get("client")
            .and_then(|c| c.get("visitorData"))
            .and_then(|v| v.as_str())
            .or_else(|| context.get("visitorData").and_then(|v| v.as_str()));

        let body = self.player_request(track_id, visitor_data, None).await?;

        if body
            .get("playabilityStatus")
            .and_then(|p| p.get("status"))
            .and_then(|s| s.as_str())
            != Some("OK")
        {
            return Ok(None);
        }

        Ok(extract_from_player(&body, "youtube"))
    }

    async fn get_playlist(
        &self,
        playlist_id: &str,
        context: &Value,
        _oauth: Arc<YouTubeOAuth>,
    ) -> AnyResult<Option<(Vec<Track>, String)>> {
        let visitor_data = context
            .get("client")
            .and_then(|c| c.get("visitorData"))
            .and_then(|v| v.as_str())
            .or_else(|| context.get("visitorData").and_then(|v| v.as_str()));

        let body = self
            .next_request(None, Some(playlist_id), visitor_data)
            .await?;

        Ok(extract_from_next(&body, "youtube"))
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
        _oauth: Arc<YouTubeOAuth>,
    ) -> AnyResult<Option<String>> {
        let visitor_data = context
            .get("client")
            .and_then(|c| c.get("visitorData"))
            .and_then(|v| v.as_str())
            .or_else(|| context.get("visitorData").and_then(|v| v.as_str()));

        let signature_timestamp = cipher_manager.get_signature_timestamp().await.ok();
        let body = self
            .player_request(track_id, visitor_data, signature_timestamp)
            .await?;

        let playability = body
            .get("playabilityStatus")
            .and_then(|p| p.get("status"))
            .and_then(|s| s.as_str())
            .unwrap_or("UNKNOWN");

        if playability != "OK" {
            tracing::warn!(
                "TV Cast player: video {} not playable (status={})",
                track_id,
                playability
            );
            return Ok(None);
        }

        let streaming_data = match body.get("streamingData") {
            Some(sd) => sd,
            None => {
                tracing::error!("TV Cast player: no streamingData for {}", track_id);
                return Ok(None);
            }
        };

        if let Some(hls) = streaming_data
            .get("hlsManifestUrl")
            .and_then(|v| v.as_str())
        {
            tracing::debug!("TV Cast player: using HLS manifest for {}", track_id);
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
                        "TV Cast player: best format had no resolvable URL for {}",
                        track_id
                    );
                }
                Err(e) => {
                    tracing::error!(
                        "TV Cast player: cipher resolution failed for {}: {}",
                        track_id,
                        e
                    );
                    return Err(e);
                }
            }
        }

        Ok(None)
    }

    async fn get_player_body(
        &self,
        track_id: &str,
        visitor_data: Option<&str>,
        _oauth: Arc<YouTubeOAuth>,
    ) -> Option<serde_json::Value> {
        self.player_request(track_id, visitor_data, None)
            .await
            .ok()
    }
}
