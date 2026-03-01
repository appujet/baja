use std::sync::Arc;

use async_trait::async_trait;
use regex::Regex;
use tracing::debug;

use crate::{
    configs::sources::GoogleTtsConfig,
    protocol::tracks::{LoadResult, Track, TrackInfo},
    sources::{
        http::HttpTrack,
        plugin::{BoxedTrack, SourcePlugin},
    },
};

pub struct GoogleTtsSource {
    config: GoogleTtsConfig,
    search_prefixes: Vec<String>,
    url_pattern: Regex,
}

impl GoogleTtsSource {
    pub fn new(config: GoogleTtsConfig) -> Self {
        Self {
            config,
            search_prefixes: vec!["gtts:".to_string(), "speak:".to_string()],
            url_pattern: Regex::new(r"(?i)^(gtts://|speak://)").unwrap(),
        }
    }

    fn build_track_info(&self, language: &str, text: &str) -> TrackInfo {
        let title_text = if text.len() > 50 {
            format!("{}...", &text[..47])
        } else {
            text.to_string()
        };

        TrackInfo {
            identifier: format!("gtts://{}:{}", language, text),
            is_seekable: true,
            author: "Google TTS".to_string(),
            length: 0, // length is unknown/unlimited
            is_stream: false,
            position: 0,
            title: format!("TTS: {}", title_text),
            uri: Some(self.build_url(language, text)),
            source_name: self.name().to_string(),
            artwork_url: None,
            isrc: None,
        }
    }

    fn build_url(&self, language: &str, text: &str) -> String {
        let encoded_text = urlencoding::encode(text);
        format!(
            "https://translate.google.com/translate_tts?ie=UTF-8&q={}&tl={}&total=1&idx=0&textlen={}&client=gtx",
            encoded_text,
            language,
            text.len()
        )
    }

    fn parse_query(&self, identifier: &str) -> (String, String) {
        let mut path = identifier;

        for prefix in &self.search_prefixes {
            if path.starts_with(prefix) {
                path = path.trim_start_matches(prefix);
                break;
            }
        }

        // Handle // after prefix (e.g., gtts://en:hello)
        if path.starts_with("//") {
            path = &path[2..];
        }

        if let Some(split_idx) = path.find(':') {
            let lang = &path[..split_idx];
            let actual_text = &path[split_idx + 1..];
            if !lang.is_empty() {
                return (lang.to_string(), actual_text.to_string());
            }
        }

        (self.config.language.clone(), path.to_string())
    }
}

#[async_trait]
impl SourcePlugin for GoogleTtsSource {
    fn name(&self) -> &str {
        "google-tts"
    }

    fn can_handle(&self, identifier: &str) -> bool {
        self.search_prefixes
            .iter()
            .any(|p| identifier.starts_with(p))
            || self.url_pattern.is_match(identifier)
    }

    async fn load(
        &self,
        identifier: &str,
        _routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> LoadResult {
        debug!("Google TTS loading: {}", identifier);

        let (language, text) = self.parse_query(identifier);

        if text.trim().is_empty() {
            return LoadResult::Empty {};
        }

        let info = self.build_track_info(&language, &text);
        LoadResult::Track(Track::new(info))
    }

    async fn get_track(
        &self,
        identifier: &str,
        routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<BoxedTrack> {
        let (language, text) = self.parse_query(identifier);
        let url = self.build_url(&language, &text);

        // Use HttpTrack to decode the audio stream
        Some(Box::new(HttpTrack {
            url,
            local_addr: routeplanner.and_then(|rp| rp.get_address()),
            proxy: None, // Google TTS doesn't currently support proxy config directly in the new implementation, similar to Spotify
        }))
    }

    fn search_prefixes(&self) -> Vec<&str> {
        self.search_prefixes.iter().map(|s| s.as_str()).collect()
    }
}
