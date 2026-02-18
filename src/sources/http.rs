use crate::api::tracks::{LoadError, LoadResult, Track, TrackInfo};
use crate::sources::SourcePlugin;
use async_trait::async_trait;
use regex::Regex;
use reqwest::header::{CONTENT_LENGTH, CONTENT_TYPE, HeaderMap};
use std::sync::Arc;
use tracing::debug;

pub struct HttpSource {
    url_regex: Regex,
    client: reqwest::Client,
}

impl HttpSource {
    pub fn new() -> Self {
        Self {
            url_regex: Regex::new(r"^https?://").unwrap(),
            client: crate::common::http::HttpClient::new().unwrap(),
        }
    }

    fn is_valid_content_type(&self, content_type: &str) -> bool {
        content_type.starts_with("audio/")
            || content_type.starts_with("video/")
            || content_type == "application/octet-stream"
            || content_type.is_empty()
    }

    fn extract_metadata(&self, url: &str, headers: &HeaderMap) -> TrackInfo {
        let is_stream =
            headers.contains_key("icy-metaint") || !headers.contains_key(CONTENT_LENGTH);

        // Extract title from headers or URL
        let title = headers
            .get("icy-name")
            .and_then(|h| h.to_str().ok())
            .or_else(|| {
                headers
                    .get("content-disposition")
                    .and_then(|h| h.to_str().ok())
                    .and_then(|s| s.split("filename=\"").nth(1))
                    .and_then(|s| s.split('"').next())
            })
            .unwrap_or_else(|| {
                url.split('/')
                    .last()
                    .and_then(|s| s.split('?').next())
                    .unwrap_or("Audio Stream")
            })
            .to_string();

        let author = headers
            .get("icy-description")
            .and_then(|h| h.to_str().ok())
            .unwrap_or("unknown")
            .to_string();

        let length = if is_stream {
            u64::MAX // Infinite for streams
        } else {
            headers
                .get(CONTENT_LENGTH)
                .and_then(|h| h.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0)
        };

        let mut artwork_url = None;
        if url.starts_with("https://cdn.discordapp.com") {
            if let Some(ct) = headers.get(CONTENT_TYPE).and_then(|v| v.to_str().ok()) {
                if ct.contains("video/") {
                    let clean_url = url.split('&').next().unwrap_or(url);
                    let base = clean_url
                        .replace("https://cdn.discordapp.com", "https://media.discordapp.net");
                    let separator = if base.contains('?') { "&" } else { "?" };
                    artwork_url = Some(format!("{}{}{}", base, separator, "format=webp"));
                }
            }
        }

        TrackInfo {
            identifier: url.to_string(),
            is_seekable: !is_stream,
            author,
            length,
            is_stream,
            position: 0,
            title,
            uri: Some(url.to_string()),
            source_name: "http".to_string(),
            artwork_url,
            isrc: None,
        }
    }
}

#[async_trait]
impl SourcePlugin for HttpSource {
    fn name(&self) -> &str {
        "http"
    }

    fn can_handle(&self, identifier: &str) -> bool {
        self.url_regex.is_match(identifier)
    }

    async fn load(
        &self,
        identifier: &str,
        routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> LoadResult {
        debug!("Probing HTTP source: {}", identifier);
        
        // Note: We deliberately do NOT cache clients here when a RoutePlanner is used.
        // RoutePlanners with large IPv6 blocks generate billions of unique IPs.
        // Caching clients keyed by IP would lead to an unbounded memory leak.
        // For /64 blocks, creating a new client per request is the safe, albeit slightly slower, approach.
        let client = if let Some(rp) = routeplanner {
            if let Some(ip) = rp.get_address() {
                debug!("Using rotated IP: {}", ip);
                reqwest::Client::builder()
                    .user_agent(crate::common::http::HttpClient::random_user_agent())
                    .timeout(std::time::Duration::from_secs(10))
                    .local_address(ip)
                    .build()
                    .unwrap_or(self.client.clone())
            } else {
                self.client.clone()
            }
        } else {
            self.client.clone()
        };

        let mut resp = match client.head(identifier).send().await {
            Ok(r) => Some(r),
            Err(_) => None, // Fallback to GET
        };
        
        if resp
            .as_ref()
            .map(|r| !r.status().is_success())
            .unwrap_or(true)
        {
            match client
                .get(identifier)
                .header("Range", "bytes=0-0")
                .send()
                .await
            {
                Ok(r) => {
                    if r.status().is_success() {
                        resp = Some(r);
                    } else {
                        return LoadResult::Error(LoadError {
                            message: format!("HTTP request failed with status: {}", r.status()),
                            severity: crate::common::Severity::Common,
                            cause: "".to_string(),
                        });
                    }
                }
                Err(e) => {
                    return LoadResult::Error(LoadError {
                        message: format!("HTTP request failed: {}", e),
                        severity: crate::common::Severity::Common,
                        cause: "".to_string(),
                    });
                }
            }
        }

        let response = resp.unwrap();
        let headers = response.headers();
        let content_type = headers
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        if !self.is_valid_content_type(content_type) {
            return LoadResult::Error(LoadError {
                message: format!("Unsupported content type: {}", content_type),
                severity: crate::common::Severity::Common,
                cause: "".to_string(),
            });
        }

        let info = self.extract_metadata(identifier, headers);

        LoadResult::Track(Track::new(info))
    }

    async fn get_playback_url(
        &self,
        identifier: &str,
        _routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<String> {
        let clean = identifier
            .trim()
            .trim_start_matches('<')
            .trim_end_matches('>');

        if self.can_handle(clean) {
            Some(clean.to_string())
        } else {
            None
        }
    }
}
