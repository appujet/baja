use std::sync::Arc;

use async_trait::async_trait;
use regex::Regex;
use tracing::{debug, warn};

use crate::{
    config::TwitchConfig,
    protocol::tracks::{LoadError, LoadResult, Track, TrackInfo},
    sources::{SourcePlugin, plugin::BoxedTrack},
};

use super::{api::TwitchGqlClient, track::TwitchTrack};

const STREAM_NAME_REGEX: &str = r"(?i)^https?://(?:www\.|go\.|m\.)?twitch\.tv/([^/]+)$";
const TWITCH_DOMAIN_REGEX: &str = r"(?i)^https?://(?:www\.|go\.|m\.)?twitch\.tv/";
const TWITCH_IMAGE_PREVIEW_URL: &str =
    "https://static-cdn.jtvnw.net/previews-ttv/live_user_%s-440x248.jpg";

pub struct TwitchSource {
    gql: Arc<TwitchGqlClient>,
    proxy: Option<crate::config::HttpProxyConfig>,
    stream_name_regex: Regex,
    twitch_domain_regex: Regex,
}

impl TwitchSource {
    pub fn new(config: TwitchConfig, client: Arc<reqwest::Client>) -> Self {
        Self {
            gql: Arc::new(TwitchGqlClient::new(client, config.client_id)),
            proxy: config.proxy,
            stream_name_regex: Regex::new(STREAM_NAME_REGEX).unwrap(),
            twitch_domain_regex: Regex::new(TWITCH_DOMAIN_REGEX).unwrap(),
        }
    }

    fn get_channel_identifier_from_url(&self, url: &str) -> Option<String> {
        self.stream_name_regex
            .captures(url)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().to_lowercase())
    }

    async fn ensure_initialized(&self) {
        if !self.gql.is_initialized() {
            self.gql.init_request_headers().await;
        }
    }

    async fn get_channel_streams_url(&self, channel: &str) -> Option<String> {
        let (token, sig) = self.gql.fetch_access_token(channel).await?;

        Some(format!(
            "https://usher.ttvnw.net/api/channel/hls/{}.m3u8?token={}&sig={}&allow_source=true&allow_spectre=true&allow_audio_only=true&player_backend=html5&expgroup=regular",
            channel,
            urlencoding::encode(&token),
            urlencoding::encode(&sig),
        ))
    }

    async fn fetch_segment_playlist_url(&self, channel: &str) -> Option<String> {
        let streams_url = self.get_channel_streams_url(channel).await?;
        let m3u8 = self.gql.fetch_text(&streams_url).await?;

        let streams = load_channel_streams_list(&m3u8);
        if streams.is_empty() {
            debug!("Twitch: no streams available on channel '{channel}'");
            return None;
        }

        let chosen = streams.last().unwrap();
        debug!("Twitch: chose stream with quality {} from url {}", chosen.quality, chosen.url);
        Some(chosen.url.clone())
    }
}

struct ChannelStreamInfo {
    quality: String,
    url: String,
}

fn load_channel_streams_list(m3u8: &str) -> Vec<ChannelStreamInfo> {
    let lines: Vec<&str> = m3u8.lines().collect();
    let mut streams = Vec::new();

    let mut i = 0;
    while i < lines.len() {
        let line = lines[i].trim();
        if line.starts_with("#EXT-X-STREAM-INF:") {
            let quality = line
                .split(',')
                .find_map(|part| part.trim().strip_prefix("VIDEO=\"").and_then(|v| v.strip_suffix('"')))
                .unwrap_or("unknown")
                .to_string();

            if let Some(url_line) = lines.get(i + 1) {
                let url = url_line.trim();
                if !url.is_empty() && !url.starts_with('#') {
                    streams.push(ChannelStreamInfo { quality, url: url.to_string() });
                }
            }
        }
        i += 1;
    }

    streams
}

#[async_trait]
impl SourcePlugin for TwitchSource {
    fn name(&self) -> &str {
        "twitch"
    }

    fn can_handle(&self, identifier: &str) -> bool {
        self.twitch_domain_regex.is_match(identifier)
    }

    async fn load(
        &self,
        identifier: &str,
        _routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> LoadResult {
        let stream_name = match self.get_channel_identifier_from_url(identifier) {
            Some(n) => n,
            None => return LoadResult::Empty {},
        };

        self.ensure_initialized().await;

        let channel_info_body = match self.gql.fetch_stream_channel_info(&stream_name).await {
            Some(b) => b,
            None => {
                return LoadResult::Error(LoadError {
                    message: Some(format!("Loading Twitch channel information failed for '{stream_name}'")),
                    severity: crate::common::Severity::Suspicious,
                    cause: "GQL request failed".to_string(),
                    cause_stack_trace: None,
                });
            }
        };

        let channel_info = &channel_info_body["data"]["user"];

        if channel_info.is_null() || channel_info["stream"]["type"].is_null() {
            return LoadResult::Empty {};
        }

        let title = channel_info["lastBroadcast"]["title"]
            .as_str()
            .unwrap_or("")
            .to_string();

        let thumbnail = TWITCH_IMAGE_PREVIEW_URL.replace("%s", &stream_name);

        LoadResult::Track(Track::new(TrackInfo {
            identifier: stream_name.clone(),
            is_seekable: false,
            author: stream_name.clone(),
            length: 0,
            is_stream: true,
            position: 0,
            title,
            uri: Some(identifier.to_string()),
            artwork_url: Some(thumbnail),
            isrc: None,
            source_name: "twitch".to_string(),
        }))
    }

    async fn get_track(
        &self,
        identifier: &str,
        routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<BoxedTrack> {
        let stream_name = self.get_channel_identifier_from_url(identifier)?;

        self.ensure_initialized().await;

        let local_addr = routeplanner.and_then(|rp| rp.get_address());

        let stream_url = match self.fetch_segment_playlist_url(&stream_name).await {
            Some(u) => u,
            None => {
                warn!("Twitch: failed to resolve stream for '{stream_name}'");
                return None;
            }
        };

        Some(Box::new(TwitchTrack {
            stream_url,
            local_addr,
            proxy: self.proxy.clone(),
        }))
    }

    fn get_proxy_config(&self) -> Option<crate::config::HttpProxyConfig> {
        self.proxy.clone()
    }
}
