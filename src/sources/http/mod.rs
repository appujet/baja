pub mod reader;
use std::sync::Arc;

use async_trait::async_trait;
use regex::Regex;
use symphonia::core::{
    codecs::CODEC_TYPE_NULL,
    formats::FormatOptions,
    io::MediaSourceStream,
    meta::{MetadataOptions, StandardTagKey},
    probe::Hint,
};
use tracing::{debug, error, warn};

use crate::{
    audio::processor::{AudioProcessor, DecoderCommand},
    common::types::AnyResult,
    protocol::tracks::{LoadError, LoadResult, Track, TrackInfo},
    sources::{SourcePlugin, plugin::PlayableTrack},
};

pub struct HttpSource {
    url_regex: Regex,
}

impl HttpSource {
    pub fn new() -> Self {
        Self {
            url_regex: Regex::new(r"^(?:https?|icy)://").unwrap(),
        }
    }

    fn probe_metadata(url: String, local_addr: Option<std::net::IpAddr>) -> AnyResult<TrackInfo> {
        let source = self::reader::HttpReader::new(&url, local_addr, None)?;
        let mut hint = Hint::new();

        if let Some(content_type) = source.content_type() {
            hint.mime_type(&content_type);
        }

        let mss = MediaSourceStream::new(Box::new(source), Default::default());

        if let Some(ext) = std::path::Path::new(&url)
            .extension()
            .and_then(|s| s.to_str())
        {
            hint.with_extension(ext);
        }

        let probed = symphonia::default::get_probe().format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )?;

        let mut format = probed.format;
        let track = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or("no audio track found")?;

        // Calculate duration safely
        let duration = if let Some(n_frames) = track.codec_params.n_frames {
            if let Some(rate) = track.codec_params.sample_rate {
                (n_frames as f64 / rate as f64 * 1000.0) as u64
            } else {
                0
            }
        } else {
            0
        };

        // Extract metadata
        let mut title = String::new();
        let mut author = String::new();

        if let Some(metadata) = format.metadata().current() {
            if let Some(tag) = metadata
                .tags()
                .iter()
                .find(|t| t.std_key == Some(StandardTagKey::TrackTitle))
            {
                title = tag.value.to_string();
            }
            if let Some(tag) = metadata
                .tags()
                .iter()
                .find(|t| t.std_key == Some(StandardTagKey::Artist))
            {
                author = tag.value.to_string();
            }
        }

        // Fallback metadata from URL if tags are missing
        if title.is_empty() {
            title = url
                .split('/')
                .last()
                .and_then(|s| s.split('?').next())
                .unwrap_or("Unknown Title")
                .to_string();
        }
        if author.is_empty() {
            author = "Unknown Artist".to_string();
        }

        Ok(TrackInfo {
            identifier: url.clone(),
            is_seekable: true, // Symphonia sources are generally seekable if the container supports it
            author,
            length: duration,
            is_stream: false, // If we probed it successfully, it's likely a file/VOD
            position: 0,
            title,
            uri: Some(url),
            source_name: "http".to_string(),
            artwork_url: None,
            isrc: None,
        })
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

        let identifier = identifier.to_string();
        let local_addr = routeplanner.as_ref().and_then(|rp| rp.get_address());

        let identifier_clone = identifier.clone();
        // Probe in a blocking task to avoid blocking the async runtime
        let probe_result = tokio::task::spawn_blocking(move || {
            HttpSource::probe_metadata(identifier_clone, local_addr)
        })
        .await;

        match probe_result {
            Ok(Ok(info)) => LoadResult::Track(Track::new(info)),
            Ok(Err(e)) => {
                warn!("Probing failed for {}: {}", identifier, e);
                // If probing fails (e.g. not an audio file), we should return Empty
                // so the manager knows no track was found, rather than erroring.
                // This mimics Lavaplayer's behavior where unknown formats return null.
                LoadResult::Empty {}
            }
            Err(e) => {
                error!("Task join error: {}", e);
                LoadResult::Error(LoadError {
                    message: Some("Internal error during probing".to_string()),
                    severity: crate::common::Severity::Suspicious,
                    cause: e.to_string(),
                    cause_stack_trace: None,
                })
            }
        }
    }

    async fn get_track(
        &self,
        identifier: &str,
        routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<Box<dyn PlayableTrack>> {
        let clean = identifier
            .trim()
            .trim_start_matches('<')
            .trim_end_matches('>');

        if self.can_handle(clean) {
            Some(Box::new(HttpTrack {
                url: clean.to_string(),
                local_addr: routeplanner.and_then(|rp| rp.get_address()),
                proxy: None,
            }))
        } else {
            None
        }
    }
}

pub struct HttpTrack {
    pub url: String,
    pub local_addr: Option<std::net::IpAddr>,
    pub proxy: Option<crate::configs::HttpProxyConfig>,
}

impl PlayableTrack for HttpTrack {
    fn start_decoding(
        &self,
        config: crate::configs::player::PlayerConfig,
    ) -> (
        flume::Receiver<crate::audio::buffer::PooledBuffer>,
        flume::Sender<DecoderCommand>,
        flume::Receiver<String>,
        Option<flume::Receiver<std::sync::Arc<Vec<u8>>>>,
    ) {
        let (tx, rx) = flume::bounded::<crate::audio::buffer::PooledBuffer>(
            (config.buffer_duration_ms / 20) as usize,
        );
        let (cmd_tx, cmd_rx) = flume::unbounded::<DecoderCommand>();
        let (err_tx, err_rx) = flume::bounded::<String>(1);

        let url = self.url.clone();
        let local_addr = self.local_addr;
        let proxy = self.proxy.clone();

        let handle = tokio::runtime::Handle::current();
        std::thread::spawn(move || {
            let _guard = handle.enter();
            let reader = match self::reader::HttpReader::new(&url, local_addr, proxy) {
                Ok(r) => Box::new(r) as Box<dyn symphonia::core::io::MediaSource>,
                Err(e) => {
                    error!("Failed to create HttpReader for HTTP: {}", e);
                    let _ = err_tx.send(format!("Failed to open stream: {}", e));
                    return;
                }
            };

            let kind = std::path::Path::new(&url)
                .extension()
                .and_then(|s| s.to_str())
                .map(crate::common::types::AudioFormat::from_ext);

            match AudioProcessor::new(
                reader,
                kind,
                tx,
                cmd_rx,
                Some(err_tx.clone()),
                config.clone(),
            ) {
                Ok(mut processor) => {
                    if let Err(e) = processor.run() {
                        error!("HTTP track audio processor error: {}", e);
                    }
                }
                Err(e) => {
                    error!("HTTP track failed to initialize processor: {}", e);
                    let _ = err_tx.send(format!("Failed to initialize processor: {}", e));
                }
            }
        });

        (rx, cmd_tx, err_rx, None)
    }
}
