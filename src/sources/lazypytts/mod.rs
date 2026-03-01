use std::sync::Arc;

use async_trait::async_trait;
use regex::Regex;
use serde::Deserialize;
use tracing::{debug, error};

use crate::{
    audio::processor::{AudioProcessor, DecoderCommand},
    configs::sources::LazyPyTtsConfig,
    protocol::tracks::{LoadError, LoadResult, Track, TrackInfo},
    sources::{
        http::reader::HttpReader,
        plugin::{BoxedTrack, PlayableTrack, SourcePlugin},
    },
};

pub struct LazyPyTtsSource {
    config: LazyPyTtsConfig,
    search_prefixes: Vec<String>,
    url_pattern: Regex,
}

impl LazyPyTtsSource {
    pub fn new(config: LazyPyTtsConfig) -> Self {
        Self {
            config,
            search_prefixes: vec!["lazypytts:".to_string(), "lazytts:".to_string()],
            url_pattern: Regex::new(r"(?i)^(lazypytts://|lazytts://)").unwrap(),
        }
    }

    fn parse_query(&self, identifier: &str) -> (String, String, String) {
        let mut raw = identifier;

        for prefix in &self.search_prefixes {
            if raw.starts_with(prefix) {
                raw = raw.trim_start_matches(prefix);
                break;
            }
        }

        // Handle // after prefix (e.g., lazypytts://hello)
        if raw.starts_with("//") {
            raw = &raw[2..];
        }

        raw = raw.trim();

        let parts: Vec<&str> = raw.split(':').collect();

        // format: service:voice:text
        if parts.len() >= 3 {
            let service = parts[0];
            let voice = parts[1];
            let text = parts[2..].join(":");
            return (service.to_string(), voice.to_string(), text);
        }

        // format: voice:text (use config default service)
        if parts.len() >= 2 {
            let voice = parts[0];
            let text = parts[1..].join(":");
            return (self.config.service.clone(), voice.to_string(), text);
        }

        // format: text only (use config default service and voice)
        (
            self.config.service.clone(),
            self.config.voice.clone(),
            raw.to_string(),
        )
    }

    fn build_track_info(
        &self,
        _service: &str,
        voice: &str,
        text: &str,
        identifier: &str,
    ) -> TrackInfo {
        let title_text = if text.len() > 50 {
            format!("{}...", &text[..47])
        } else {
            text.to_string()
        };

        TrackInfo {
            identifier: identifier.to_string(),
            is_seekable: true,
            author: "LazyPy TTS".to_string(),
            length: 0,
            is_stream: false,
            position: 0,
            title: format!("TTS ({}): {}", voice, title_text),
            uri: Some(identifier.to_string()),
            source_name: self.name().to_string(),
            artwork_url: None,
            isrc: None,
        }
    }
}

#[async_trait]
impl SourcePlugin for LazyPyTtsSource {
    fn name(&self) -> &str {
        "lazypytts"
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
        debug!("LazyPy TTS loading: {}", identifier);

        let (service, voice, text) = self.parse_query(identifier);

        if text.trim().is_empty() {
            return LoadResult::Empty {};
        }

        if text.len() > self.config.max_text_length {
            return LoadResult::Error(LoadError {
                message: Some(format!(
                    "Text too long for LazyPy TTS. Max {} characters.",
                    self.config.max_text_length
                )),
                severity: crate::common::Severity::Common,
                cause: "Text too long".to_string(),
                cause_stack_trace: None,
            });
        }

        let info = self.build_track_info(&service, &voice, &text, identifier);
        LoadResult::Track(Track::new(info))
    }

    async fn get_track(
        &self,
        identifier: &str,
        local_addr: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<BoxedTrack> {
        let (service, voice, text) = self.parse_query(identifier);

        Some(Box::new(LazyPyTtsTrack {
            service,
            voice,
            text,
            http_client: reqwest::Client::new(),
            local_addr: local_addr.and_then(|rp| rp.get_address()),
        }))
    }

    fn search_prefixes(&self) -> Vec<&str> {
        self.search_prefixes.iter().map(|s| s.as_str()).collect()
    }
}

pub struct LazyPyTtsTrack {
    pub service: String,
    pub voice: String,
    pub text: String,
    pub http_client: reqwest::Client,
    pub local_addr: Option<std::net::IpAddr>,
}

#[derive(Deserialize)]
struct LazyPyResponse {
    success: bool,
    audio_url: Option<String>,
    error_msg: Option<String>,
}

impl PlayableTrack for LazyPyTtsTrack {
    fn start_decoding(
        &self,
        config: crate::configs::player::PlayerConfig,
    ) -> (
        flume::Receiver<crate::audio::buffer::PooledBuffer>,
        flume::Sender<DecoderCommand>,
        flume::Receiver<String>,
        Option<flume::Receiver<std::sync::Arc<Vec<u8>>>>,
    ) {
        let (tx, rx) = flume::bounded((config.buffer_duration_ms / 20) as usize);
        let (cmd_tx, cmd_rx) = flume::unbounded();
        let (err_tx, err_rx) = flume::bounded(1);

        let service = self.service.clone();
        let voice = self.voice.clone();
        let text = self.text.clone();
        let http_client = self.http_client.clone();
        let local_addr = self.local_addr;
        let config_clone = config.clone();

        let handle = tokio::runtime::Handle::current();
        std::thread::spawn(move || {
            let _guard = handle.enter();

            // Run async request to get audio URL
            let result = handle.block_on(async {
                let params = [("service", service), ("voice", voice), ("text", text)];

                let res = http_client
                    .post("https://lazypy.ro/tts/request_tts.php")
                    .header("Accept", "*/*")
                    .header("Content-Type", "application/x-www-form-urlencoded")
                    .header("Origin", "https://lazypy.ro")
                    .header("Referer", "https://lazypy.ro/tts/")
                    .header("User-Agent", "LazyPyTTS")
                    .form(&params)
                    .send()
                    .await;

                let response = match res {
                    Ok(r) => r,
                    Err(e) => return Err(format!("Request failed: {}", e)),
                };

                let success_status = response.status().is_success();
                let txt = match response.text().await {
                    Ok(t) => t,
                    Err(e) => return Err(format!("Failed reading response: {}", e)),
                };

                if !success_status {
                    return Err(format!("LazyPy TTS returned status {}", success_status));
                }

                let payload: LazyPyResponse = match serde_json::from_str(&txt) {
                    Ok(p) => p,
                    Err(_) => return Err("Failed to parse LazyPy TTS JSON response".to_string()),
                };

                if !payload.success {
                    return Err(payload
                        .error_msg
                        .unwrap_or_else(|| "LazyPy TTS request failed.".to_string()));
                }

                payload
                    .audio_url
                    .ok_or_else(|| "No audio URL in response".to_string())
            });

            let final_url = match result {
                Ok(url) => url,
                Err(e) => {
                    error!("LazyPy TTS error: {}", e);
                    let _ = err_tx.send(e);
                    return;
                }
            };

            let reader = match HttpReader::new(&final_url, local_addr, None) {
                Ok(r) => Box::new(r) as Box<dyn symphonia::core::io::MediaSource>,
                Err(e) => {
                    error!("Failed to create HttpReader for LazyPy TTS: {}", e);
                    let _ = err_tx.send(format!("Failed to open stream: {}", e));
                    return;
                }
            };

            let kind = std::path::Path::new(&final_url)
                .extension()
                .and_then(|s| s.to_str())
                .map(crate::common::types::AudioFormat::from_ext);

            match AudioProcessor::new(reader, kind, tx, cmd_rx, Some(err_tx.clone()), config_clone)
            {
                Ok(mut processor) => {
                    if let Err(e) = processor.run() {
                        error!("LazyPy TTS track audio processor error: {}", e);
                    }
                }
                Err(e) => {
                    error!("LazyPy TTS track failed to initialize processor: {}", e);
                    let _ = err_tx.send(format!("Failed to initialize processor: {}", e));
                }
            }
        });

        (rx, cmd_tx, err_rx, None)
    }
}
