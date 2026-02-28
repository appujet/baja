use std::sync::Arc;
use async_trait::async_trait;
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
}

impl GoogleTtsSource {
    pub fn new(config: GoogleTtsConfig) -> Self {
        Self {
            config,
            search_prefixes: vec!["gtts:".to_string(), "speak:".to_string()],
        }
    }

    fn build_track_info(&self, text: &str) -> TrackInfo {
        let title_text = if text.len() > 50 {
            format!("{}...", &text[..47])
        } else {
            text.to_string()
        };

        TrackInfo {
            identifier: format!("gtts:{}", text),
            is_seekable: true,
            author: "Google TTS".to_string(),
            length: 0, // length is unknown/unlimited
            is_stream: false,
            position: 0,
            title: format!("TTS: {}", title_text),
            uri: Some(self.build_url(text)),
            source_name: self.name().to_string(),
            artwork_url: None,
            isrc: None,
        }
    }

    fn build_url(&self, text: &str) -> String {
        let encoded_text = urlencoding::encode(text);
        format!(
            "https://translate.google.com/translate_tts?ie=UTF-8&q={}&tl={}&total=1&idx=0&textlen={}&client=gtx",
            encoded_text,
            self.config.language,
            text.len()
        )
    }
}

#[async_trait]
impl SourcePlugin for GoogleTtsSource {
    fn name(&self) -> &str {
        "google-tts"
    }

    fn can_handle(&self, identifier: &str) -> bool {
        self.search_prefixes.iter().any(|p| identifier.starts_with(p))
    }

    async fn load(
        &self,
        identifier: &str,
        _routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> LoadResult {
        debug!("Google TTS loading: {}", identifier);
        
        let text = if identifier.starts_with("gtts:") {
            identifier.trim_start_matches("gtts:")
        } else if identifier.starts_with("speak:") {
            identifier.trim_start_matches("speak:")
        } else {
            identifier
        };

        if text.trim().is_empty() {
            return LoadResult::Empty {};
        }

        let info = self.build_track_info(text);
        LoadResult::Track(Track::new(info))
    }

    async fn get_track(
        &self,
        identifier: &str,
        routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<BoxedTrack> {
        let text = if identifier.starts_with("gtts:") {
            identifier.trim_start_matches("gtts:")
        } else if identifier.starts_with("speak:") {
            identifier.trim_start_matches("speak:")
        } else {
            identifier
        };
        
        let url = self.build_url(text);
        
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
