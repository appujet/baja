use crate::rest::models::*;
use crate::sources::SourcePlugin;
use async_trait::async_trait;
use base64::prelude::*;
use regex::Regex;

/// HTTP/HTTPS Source Plugin
///
/// Handles direct audio stream URLs.
/// Supports both http:// and https:// protocols.
pub struct HttpSource {
    url_regex: Regex,
}

impl HttpSource {
    pub fn new() -> Self {
        Self {
            // Matches http:// or https:// URLs
            url_regex: Regex::new(r"^https?://").unwrap(),
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
        let encoded = BASE64_STANDARD.encode(identifier.as_bytes());

        // Extract filename from URL for title
        let title = identifier
            .split('/')
            .last()
            .and_then(|s| s.split('?').next())
            .unwrap_or("Audio Stream")
            .to_string();

        LoadTracksResponse {
            load_type: LoadType::Track,
            data: LoadData::Track(Track {
                encoded,
                info: TrackInfo {
                    identifier: identifier.to_string(),
                    is_seekable: true,
                    author: "Direct Link".to_string(),
                    length: 0,
                    is_stream: true,
                    position: 0,
                    title,
                    uri: identifier.to_string(),
                    source_name: "http".to_string(),
                    artwork_url: None,
                    isrc: None,
                },
            }),
        }
    }

    async fn get_playback_url(&self, identifier: &str) -> Option<String> {
        // For HTTP sources, the identifier IS the playback URL
        // Just clean it up
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
