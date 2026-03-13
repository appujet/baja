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
        cipher::YouTubeCipherManager,
        clients::common::ClientConfig,
        extractor::{extract_from_player, extract_track, find_section_list},
        oauth::YouTubeOAuth,
    },
};

const CLIENT_NAME: &str = "WEB";
const CLIENT_ID: &str = "1";
const CLIENT_VERSION: &str = "2.20260114.01.00";
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) \
     AppleWebKit/537.36 (KHTML, like Gecko) Chrome/143.0.0.0 Safari/537.36";

pub struct WebClient {
    http: Arc<reqwest::Client>,
    /// Optional URL of the yt-cipher service for SABR PoToken fetching.
    pub yt_cipher_url: Option<String>,
    /// Optional API token for the yt-cipher service (matches its API_TOKEN env var).
    pub yt_cipher_token: Option<String>,
}

impl WebClient {
    pub fn new(http: Arc<reqwest::Client>) -> Self {
        Self {
            http,
            yt_cipher_url: None,
            yt_cipher_token: None,
        }
    }

    /// Configure the yt-cipher service URL and optional API token.
    pub fn with_cipher_url(
        http: Arc<reqwest::Client>,
        yt_cipher_url: Option<String>,
        yt_cipher_token: Option<String>,
    ) -> Self {
        let mut client = Self::new(http);
        client.yt_cipher_url = yt_cipher_url;
        client.yt_cipher_token = yt_cipher_token;
        client
    }

    fn config(&self) -> ClientConfig<'_> {
        ClientConfig {
            client_name: CLIENT_NAME,
            client_version: CLIENT_VERSION,
            client_id: CLIENT_ID,
            user_agent: USER_AGENT,
            platform: Some("DESKTOP"),
            ..Default::default()
        }
    }

    async fn player_request(
        &self,
        video_id: &str,
        visitor_data: Option<&str>,
        signature_timestamp: Option<u32>,
        _oauth: &Arc<YouTubeOAuth>,
        po_token: Option<&str>,
    ) -> AnyResult<Value> {
        crate::sources::youtube::clients::common::make_player_request(
            crate::sources::youtube::clients::common::PlayerRequestOptions {
                http: &self.http,
                config: &self.config(),
                video_id,
                params: None,
                visitor_data,
                signature_timestamp,
                auth_header: None,
                referer: None,
                origin: None,
                po_token,
                encrypted_host_flags: None,
                serialized_third_party_embed_config: false,
            },
        )
        .await
    }
}

#[async_trait]
impl YouTubeClient for WebClient {
    fn name(&self) -> &str {
        "Web"
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

        let _ = oauth;

        let res = req.send().await?;
        if !res.status().is_success() {
            return Err(format!("Web search failed: {}", res.status()).into());
        }

        let response: Value = res.json().await?;
        let mut tracks = Vec::new();

        if let Some(section_list) = find_section_list(&response)
            && let Some(contents) = section_list.get("contents").and_then(|c| c.as_array())
        {
            for section in contents {
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

        if tracks.is_empty() {
            tracing::debug!("Web search returned no tracks for query: {}", query);
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
            .player_request(track_id, visitor_data, None, &oauth, None)
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
            "context": self.config().build_context(visitor_data),
            "playlistId": playlist_id,
            "contentCheckOk": true,
            "racyCheckOk": true
        });

        let url = format!("{}/youtubei/v1/next?prettyPrint=false", INNERTUBE_API);

        let mut req = self
            .http
            .post(&url)
            .header("User-Agent", USER_AGENT)
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

        // Web client requires cipher resolution (signatureTimestamp / n-param).
        let signature_timestamp = cipher_manager.get_signature_timestamp().await.ok();
        let body = self
            .player_request(
                track_id,
                visitor_data,
                signature_timestamp,
                &oauth,
                None,
            )
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
                "Web player: video {} not playable (status={}, reason={}); attempting streamingData fallback",
                track_id,
                playability,
                reason
            );
        }

        let streaming_data = match body.get("streamingData") {
            Some(sd) => sd,
            None => {
                tracing::error!("Web player: no streamingData for {}", track_id);
                return Ok(None);
            }
        };

        // HLS for live streams
        if let Some(hls) = streaming_data
            .get("hlsManifestUrl")
            .and_then(|v| v.as_str())
        {
            tracing::debug!("Web player: using HLS manifest for {}", track_id);
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
                        "Web player: resolved audio URL for {} (itag={})",
                        track_id,
                        best.get("itag").and_then(|v| v.as_i64()).unwrap_or(-1)
                    );
                    return Ok(Some(url));
                }
                Ok(None) => {
                    tracing::warn!(
                        "Web player: best format had no resolvable URL for {}",
                        track_id
                    );
                }
                Err(e) => {
                    tracing::error!(
                        "Web player: cipher resolution failed for {}: {}",
                        track_id,
                        e
                    );
                    return Err(e);
                }
            }
        }

        tracing::warn!(
            "Web player: no suitable audio format found for {}",
            track_id
        );
        Ok(None)
    }
}
