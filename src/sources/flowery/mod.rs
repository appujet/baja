use std::sync::Arc;
use async_trait::async_trait;
use regex::Regex;
use tracing::debug;

use crate::{
    configs::sources::FloweryConfig,
    protocol::tracks::{LoadResult, Track, TrackInfo},
    sources::{
        http::HttpTrack,
        plugin::{BoxedTrack, SourcePlugin},
    },
};

pub struct FlowerySource {
    config: FloweryConfig,
    search_prefixes: Vec<String>,
    url_pattern: Regex,
}

impl FlowerySource {
    pub fn new(config: FloweryConfig) -> Self {
        Self {
            config,
            search_prefixes: vec![
                "ftts:".to_string(),
            ],
            url_pattern: Regex::new(r"(?i)^ftts://").unwrap(),
        }
    }

    fn build_track_info(&self, text: &str, identifier: &str, url: &str) -> TrackInfo {
        let title_text = if text.len() > 50 {
            format!("{}...", &text[..47])
        } else {
            text.to_string()
        };

        TrackInfo {
            identifier: identifier.to_string(),
            is_seekable: true,
            author: "Flowery TTS".to_string(),
            length: 0,
            is_stream: false,
            position: 0,
            title: title_text,
            uri: Some(url.to_string()),
            source_name: self.name().to_string(),
            artwork_url: None,
            isrc: None,
        }
    }

    fn build_url(&self, text: &str, params_override: std::collections::HashMap<String, String>) -> String {
        let mut voice = self.config.voice.clone();
        let mut translate = self.config.translate;
        let mut silence = self.config.silence;
        let mut speed = self.config.speed;

        if !self.config.enforce_config {
            if let Some(v) = params_override.get("voice") {
                voice = v.clone();
            }
            if let Some(t) = params_override.get("translate") {
                translate = t.parse().unwrap_or(translate);
            }
            if let Some(s) = params_override.get("silence") {
                silence = s.parse().unwrap_or(silence);
            }
            if let Some(sp) = params_override.get("speed") {
                speed = sp.parse().unwrap_or(speed);
            }
        }

        let encoded_text = urlencoding::encode(text);
        format!(
            "https://api.flowery.pw/v1/tts?voice={}&text={}&translate={}&silence={}&audio_format=mp3&speed={}",
            urlencoding::encode(&voice),
            encoded_text,
            translate,
            silence,
            speed
        )
    }

    fn parse_query(&self, identifier: &str) -> (String, std::collections::HashMap<String, String>) {
        let mut path_and_query = identifier;

        for prefix in &self.search_prefixes {
            if path_and_query.starts_with(prefix) {
                path_and_query = path_and_query.trim_start_matches(prefix);
                break;
            }
        }

        // Handle // after prefix (e.g., ftts://hello)
        if path_and_query.starts_with("//") {
            path_and_query = &path_and_query[2..];
        }
        let mut params = std::collections::HashMap::new();

        let text = if let Some(split_idx) = path_and_query.find('?') {
            let decoded_text = urlencoding::decode(&path_and_query[..split_idx])
                .unwrap_or_else(|_| std::borrow::Cow::Borrowed(&path_and_query[..split_idx]))
                .into_owned();
            
            let query_str = &path_and_query[split_idx + 1..];
            for pair in query_str.split('&') {
                if let Some(eq_idx) = pair.find('=') {
                    let key = &pair[..eq_idx];
                    let value = &pair[eq_idx + 1..];
                    params.insert(
                        urlencoding::decode(key).unwrap_or_else(|_| std::borrow::Cow::Borrowed(key)).into_owned(),
                        urlencoding::decode(value).unwrap_or_else(|_| std::borrow::Cow::Borrowed(value)).into_owned(),
                    );
                } else if !pair.is_empty() {
                    params.insert(
                        urlencoding::decode(pair).unwrap_or_else(|_| std::borrow::Cow::Borrowed(pair)).into_owned(),
                        "".to_string(),
                    );
                }
            }
            decoded_text
        } else {
            urlencoding::decode(path_and_query)
                .unwrap_or_else(|_| std::borrow::Cow::Borrowed(path_and_query))
                .into_owned()
        };

        (text, params)
    }
}

#[async_trait]
impl SourcePlugin for FlowerySource {
    fn name(&self) -> &str {
        "flowery"
    }

    fn can_handle(&self, identifier: &str) -> bool {
        self.search_prefixes.iter().any(|p| identifier.starts_with(p))
            || self.url_pattern.is_match(identifier)
    }

    async fn load(
        &self,
        identifier: &str,
        _routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> LoadResult {
        debug!("Flowery TTS loading: {}", identifier);
        
        let (text, params) = self.parse_query(identifier);

        if text.trim().is_empty() {
            return LoadResult::Empty {};
        }

        let url = self.build_url(&text, params);
        let info = self.build_track_info(&text, identifier, &url);
        LoadResult::Track(Track::new(info))
    }

    async fn get_track(
        &self,
        identifier: &str,
        routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<BoxedTrack> {
        let (text, params) = self.parse_query(identifier);
        let url = self.build_url(&text, params);
        
        Some(Box::new(HttpTrack {
            url,
            local_addr: routeplanner.and_then(|rp| rp.get_address()),
            proxy: None,
        }))
    }
    
    fn search_prefixes(&self) -> Vec<&str> {
        self.search_prefixes.iter().map(|s| s.as_str()).collect()
    }
}
