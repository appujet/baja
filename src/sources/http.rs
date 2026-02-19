use crate::api::tracks::{LoadError, LoadResult, Track, TrackInfo};
use crate::sources::SourcePlugin;
use async_trait::async_trait;
use regex::Regex;
use std::sync::Arc;
use symphonia::core::codecs::CODEC_TYPE_NULL;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::{MetadataOptions, StandardTagKey};
use symphonia::core::probe::Hint;
use tracing::{debug, error, warn};

pub struct HttpSource {
    url_regex: Regex,
}

impl HttpSource {
    pub fn new() -> Self {
        Self {
            url_regex: Regex::new(r"^(?:https?|icy)://").unwrap(),
        }
    }

    fn probe_metadata(
        url: String,
        local_addr: Option<std::net::IpAddr>,
    ) -> Result<TrackInfo, Box<dyn std::error::Error + Send + Sync>> {
        let source = crate::audio::reader::RemoteReader::new(&url, local_addr, None)?;
        let mut hint = Hint::new();
        
        if let Some(content_type) = source.content_type() {
            hint.mime_type(&content_type);
        }

        let mss = MediaSourceStream::new(Box::new(source), Default::default());

        if let Some(ext) = std::path::Path::new(&url).extension().and_then(|s| s.to_str()) {
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
            if let Some(tag) = metadata.tags().iter().find(|t| t.std_key == Some(StandardTagKey::TrackTitle)) {
                title = tag.value.to_string();
            }
            if let Some(tag) = metadata.tags().iter().find(|t| t.std_key == Some(StandardTagKey::Artist)) {
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
                // Lavaplayer throws FriendlyException("Unknown file format") on failure.
                // We return LoadResult::Error which mimics this.
                LoadResult::Error(LoadError {
                    message: format!("Probe failed: {}", e),
                    severity: crate::common::Severity::Common,
                    cause: e.to_string(),
                })
            }
            Err(e) => {
                error!("Task join error: {}", e);
                LoadResult::Error(LoadError {
                    message: "Internal error during probing".to_string(),
                    severity: crate::common::Severity::Suspicious,
                    cause: e.to_string(),
                })
            }
        }
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
