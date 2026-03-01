use std::sync::Arc;

use async_trait::async_trait;
use flume::{Receiver, Sender};

use crate::audio::{buffer::PooledBuffer, processor::DecoderCommand};

/// A track that can start its own decoding and return audio frames.
///
/// Returns `(pcm_rx, cmd_tx, error_rx, opus_rx)` where:
/// - `pcm_rx`   — batched i16 PCM sample frames (transcode path)
/// - `cmd_tx`   — send seek/stop commands to the decoder
/// - `error_rx` — receives a single `String` if a fatal decode/IO error occurs
/// - `opus_rx`  — `Some` when the stream is already Opus (YouTube WebM passthrough);
///               raw Opus frames that bypass the PCM mix and encode entirely
pub trait PlayableTrack: Send + Sync {
    fn start_decoding(
        &self,
        config: crate::configs::player::PlayerConfig,
    ) -> (
        Receiver<PooledBuffer>,
        Sender<DecoderCommand>,
        flume::Receiver<String>,
        Option<Receiver<Arc<Vec<u8>>>>,
    );
}

/// A boxed playable track.
pub type BoxedTrack = Box<dyn PlayableTrack>;

/// A boxed source plugin.
pub type BoxedSource = Box<dyn SourcePlugin>;

/// Direct stream information returned by a source.
pub struct StreamInfo {
    /// The resolved direct URL or HLS manifest URL.
    pub url: String,
    /// The MIME type of the stream (e.g., `audio/mpeg`, `application/vnd.apple.mpegurl`).
    pub mime_type: String,
    /// The protocol: `http` or `hls`.
    pub protocol: String,
}

/// Trait that all source plugins must implement.
#[async_trait]
pub trait SourcePlugin: Send + Sync {
    /// Unique identifier for this source (e.g., "http", "youtube", "spotify")
    fn name(&self) -> &str;

    /// Check if this source can handle the given identifier.
    fn can_handle(&self, identifier: &str) -> bool;

    /// Resolve the identifier into track(s).
    async fn load(
        &self,
        identifier: &str,
        routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> crate::protocol::tracks::LoadResult;

    /// Get a playable track for the given identifier.
    async fn get_track(
        &self,
        _identifier: &str,
        _routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<BoxedTrack> {
        None
    }

    /// Search across various entities (tracks, albums, artists, etc).
    /// Corresponds to LavaSearch's loadSearch API.
    async fn load_search(
        &self,
        _query: &str,
        _types: &[String],
        _routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<crate::protocol::tracks::SearchResult> {
        None
    }

    /// Get the proxy configuration for this source, if any.
    fn get_proxy_config(&self) -> Option<crate::configs::HttpProxyConfig> {
        None
    }

    fn search_prefixes(&self) -> Vec<&str> {
        vec![]
    }

    fn isrc_prefixes(&self) -> Vec<&str> {
        vec![]
    }

    fn rec_prefixes(&self) -> Vec<&str> {
        vec![]
    }

    fn is_mirror(&self) -> bool {
        false
    }

    /// Get a direct stream URL for the given identifier, used by the /v4/trackstream endpoint.
    /// `itag` is source-specific (meaningful only for YouTube, ignored by others).
    /// Returns `None` if direct streaming is not supported by this source.
    async fn get_stream_url(
        &self,
        _identifier: &str,
        _itag: Option<i64>,
    ) -> Option<StreamInfo> {
        None
    }
}
