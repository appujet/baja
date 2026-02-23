use std::{
    io::{Read, Seek, SeekFrom},
    path::Path,
    sync::Arc,
};

use async_trait::async_trait;
use symphonia::core::{
    codecs::CODEC_TYPE_NULL,
    formats::FormatOptions,
    io::MediaSourceStream,
    meta::{MetadataOptions, StandardTagKey},
    probe::Hint,
};
use tracing::{debug, error, warn};

use crate::{
    api::tracks::{LoadError, LoadResult, Track, TrackInfo},
    audio::processor::{AudioProcessor, DecoderCommand},
    common::Severity,
    sources::{SourcePlugin, plugin::PlayableTrack},
};

pub struct LocalSource;

impl LocalSource {
    pub fn new() -> Self {
        Self
    }

    fn probe_file(path: &str) -> Result<TrackInfo, Box<dyn std::error::Error + Send + Sync>> {
        let file = std::fs::File::open(path)?;
        let ext = Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .map(|s| s.to_lowercase());

        let mut hint = Hint::new();
        if let Some(ref e) = ext {
            hint.with_extension(e);
        }

        let mss = MediaSourceStream::new(Box::new(file), Default::default());
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

        // Duration
        let duration = if let Some(n_frames) = track.codec_params.n_frames {
            if let Some(rate) = track.codec_params.sample_rate {
                (n_frames as f64 / rate as f64 * 1000.0) as u64
            } else {
                0
            }
        } else {
            0
        };

        // Metadata tags
        let mut title = String::new();
        let mut author = String::new();

        if let Some(meta) = format.metadata().current() {
            for tag in meta.tags() {
                match tag.std_key {
                    Some(StandardTagKey::TrackTitle) => title = tag.value.to_string(),
                    Some(StandardTagKey::Artist) | Some(StandardTagKey::AlbumArtist) => {
                        if author.is_empty() {
                            author = tag.value.to_string();
                        }
                    }
                    _ => {}
                }
            }
        }

        // Fallback: use the filename without extension as the title
        if title.is_empty() {
            title = Path::new(path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Unknown")
                .to_string();
        }
        if author.is_empty() {
            author = "Unknown Artist".to_string();
        }

        Ok(TrackInfo {
            identifier: path.to_string(),
            is_seekable: true,
            author,
            length: duration,
            is_stream: false,
            position: 0,
            title,
            uri: Some(format!("file://{}", path)),
            source_name: "local".to_string(),
            artwork_url: None,
            isrc: None,
        })
    }
}

#[async_trait]
impl SourcePlugin for LocalSource {
    fn name(&self) -> &str {
        "local"
    }

    fn can_handle(&self, identifier: &str) -> bool {
        let path = identifier.strip_prefix("file://").unwrap_or(identifier);
        Path::new(path).is_file()
    }

    async fn load(
        &self,
        identifier: &str,
        _routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> LoadResult {
        let path = identifier
            .strip_prefix("file://")
            .unwrap_or(identifier)
            .to_string();

        debug!("Local source probing file: {}", path);

        let path_clone = path.clone();
        let result =
            tokio::task::spawn_blocking(move || LocalSource::probe_file(&path_clone)).await;

        match result {
            Ok(Ok(info)) => LoadResult::Track(Track::new(info)),
            Ok(Err(e)) => {
                warn!("Local source: failed to probe '{}': {}", path, e);
                LoadResult::Error(LoadError {
                    message: format!("Failed to load local file: {}", e),
                    severity: Severity::Suspicious,
                    cause: e.to_string(),
                })
            }
            Err(e) => {
                error!("Local source: task join error: {}", e);
                LoadResult::Error(LoadError {
                    message: "Internal error reading local file".to_string(),
                    severity: Severity::Fault,
                    cause: e.to_string(),
                })
            }
        }
    }

    async fn get_track(
        &self,
        identifier: &str,
        _routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<Box<dyn PlayableTrack>> {
        let path = identifier
            .strip_prefix("file://")
            .unwrap_or(identifier)
            .to_string();

        if !Path::new(&path).is_file() {
            return None;
        }

        Some(Box::new(LocalTrack { path }))
    }
}

pub struct LocalTrack {
    pub path: String,
}

struct LocalFileSource {
    file: std::fs::File,
    len: u64,
}

impl LocalFileSource {
    fn open(path: &str) -> std::io::Result<Self> {
        let file = std::fs::File::open(path)?;
        let len = file.metadata()?.len();
        Ok(Self { file, len })
    }
}

impl Read for LocalFileSource {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.file.read(buf)
    }
}

impl Seek for LocalFileSource {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.file.seek(pos)
    }
}

impl symphonia::core::io::MediaSource for LocalFileSource {
    fn is_seekable(&self) -> bool {
        true
    }
    fn byte_len(&self) -> Option<u64> {
        Some(self.len)
    }
}

impl PlayableTrack for LocalTrack {
    fn start_decoding(
        &self,
    ) -> (
        flume::Receiver<Vec<i16>>,
        flume::Sender<DecoderCommand>,
        flume::Receiver<String>,
    ) {
        let (tx, rx) = flume::bounded::<Vec<i16>>(64);
        let (cmd_tx, cmd_rx) = flume::unbounded::<DecoderCommand>();
        let (err_tx, err_rx) = flume::bounded::<String>(1);

        let path = self.path.clone();

        let handle = tokio::runtime::Handle::current();
        std::thread::spawn(move || {
            let _guard = handle.enter();
            let source = match LocalFileSource::open(&path) {
                Ok(s) => Box::new(s) as Box<dyn symphonia::core::io::MediaSource>,
                Err(e) => {
                    error!("LocalTrack: failed to open '{}': {}", path, e);
                    let _ = err_tx.send(format!("Failed to open file: {}", e));
                    return;
                }
            };

            let kind = Path::new(&path)
                .extension()
                .and_then(|e| e.to_str())
                .and_then(crate::common::types::AudioKind::from_ext);

            match AudioProcessor::new(source, kind, tx, cmd_rx, Some(err_tx)) {
                Ok(mut processor) => {
                    if let Err(e) = processor.run() {
                        error!("LocalTrack audio processor error: {}", e);
                    }
                }
                Err(e) => {
                    error!("LocalTrack failed to initialize processor: {}", e);
                }
            }
        });

        (rx, cmd_tx, err_rx)
    }
}
