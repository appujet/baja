use crate::rest::models::*;
use crate::sources::SourcePlugin;
use async_trait::async_trait;
use base64::prelude::*;
use regex::Regex;
use reqwest::header::{CONTENT_LENGTH, CONTENT_TYPE, HeaderMap};
use tracing::debug;

/// HTTP/HTTPS Source Plugin
///
/// Handles direct audio stream URLs.
/// Supports both http:// and https:// protocols.
pub struct HttpSource {
    url_regex: Regex,
    client: reqwest::Client,
}

impl HttpSource {
    pub fn new() -> Self {
        Self {
            // Matches http:// or https:// URLs
            url_regex: Regex::new(r"^https?://").unwrap(),
            client: crate::utils::http::HttpClient::new().unwrap(),
        }
    }

    fn is_valid_content_type(&self, content_type: &str) -> bool {
        content_type.starts_with("audio/")
            || content_type.starts_with("video/")
            || content_type == "application/octet-stream"
            || content_type.is_empty()
    }

    fn extract_metadata(&self, url: &str, headers: &HeaderMap) -> TrackInfo {
        let is_stream = headers.contains_key("icy-metaint") || !headers.contains_key(CONTENT_LENGTH);
        
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

        // Discord CDN specific artwork handling (NodeLink parity)
        let mut artwork_url = None;
         if url.starts_with("https://cdn.discordapp.com") {
             if let Some(ct) = headers.get(CONTENT_TYPE).and_then(|v| v.to_str().ok()) {
                 if ct.contains("video/") {
                    let clean_url = url.split('&').next().unwrap_or(url);
                    let base = clean_url.replace("https://cdn.discordapp.com", "https://media.discordapp.net");
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
            uri: url.to_string(),
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

    async fn load(&self, identifier: &str) -> LoadTracksResponse {
        debug!("Probing HTTP source: {}", identifier);
        
        // 1. Try HEAD request
        let mut resp = match self.client.head(identifier).send().await {
            Ok(r) => Some(r),
            Err(_) => None, // Fallback to GET
        };

        // 2. If HEAD fails or returns bad status, try GET (stream only)
        if resp.as_ref().map(|r| !r.status().is_success()).unwrap_or(true) {
             match self.client.get(identifier).header("Range", "bytes=0-0").send().await {
                Ok(r) => {
                    if r.status().is_success() {
                        resp = Some(r);
                    } else {
                        let exception = Exception {
                            message: format!("HTTP request failed with status: {}", r.status()),
                            severity: "common".to_string(),
                            cause: "".to_string(),
                        };
                        return LoadTracksResponse {
                            load_type: LoadType::Error,
                            data: LoadData::Error(exception),
                        };
                    }
                },
                Err(e) => {
                    let exception = Exception {
                        message: format!("HTTP request failed: {}", e),
                        severity: "common".to_string(),
                        cause: "".to_string(),
                    };
                    return LoadTracksResponse {
                        load_type: LoadType::Error,
                        data: LoadData::Error(exception),
                    };
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
             let exception = Exception {
                message: format!("Unsupported content type: {}", content_type),
                severity: "common".to_string(),
                cause: "".to_string(),
            };
            return LoadTracksResponse {
                load_type: LoadType::Error,
                data: LoadData::Error(exception),
            };
        }

        let info = self.extract_metadata(identifier, headers);
        let encoded = BASE64_STANDARD.encode(identifier.as_bytes()); // Simplified encoding for now

        LoadTracksResponse {
            load_type: LoadType::Track,
            data: LoadData::Track(Track {
                encoded,
                info,
            }),
        }
    }

    async fn get_playback_url(&self, identifier: &str) -> Option<String> {
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
