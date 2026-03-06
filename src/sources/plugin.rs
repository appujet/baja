use std::sync::Arc;

use async_trait::async_trait;
use flume::{Receiver, Sender};

use crate::{
    audio::{buffer::PooledBuffer, processor::DecoderCommand},
    config::HttpProxyConfig,
    protocol::tracks::{LoadResult, SearchResult},
    routeplanner::RoutePlanner,
};

/// Represents the output of a decoder, providing streams for audio data and control commands.
///
/// Returns `(pcm_rx, cmd_tx, error_rx, opus_rx)` where:
/// - `pcm_rx`   — Receives batched i16 PCM sample frames for transcoding.
/// - `cmd_tx`   — Sends `DecoderCommand` (e.g., seek, stop) to the decoder.
/// - `error_rx` — Receives a fatal error message if decoding or IO fails.
/// - `opus_rx`  — Optional receiver for raw Opus frames (e.g., YouTube WebM passthrough).
pub type DecoderOutput = (
    Receiver<PooledBuffer>,
    Sender<DecoderCommand>,
    Receiver<String>,
    Option<Receiver<Arc<Vec<u8>>>>,
);

/// A track capable of initializing its own decoding process.
pub trait PlayableTrack: Send + Sync {
    /// Starts the decoding process with the provided player configuration.
    fn start_decoding(&self, config: crate::config::player::PlayerConfig) -> DecoderOutput;
}

pub type BoxedTrack = Box<dyn PlayableTrack>;
pub type BoxedSource = Box<dyn SourcePlugin>;

/// Core trait for all media source plugins.
#[async_trait]
pub trait SourcePlugin: Send + Sync {
    /// Returns the unique identifier for this source (e.g., "youtube", "spotify").
    fn name(&self) -> &str;

    /// Returns true if this source can handle the given identifier/URI.
    fn can_handle(&self, identifier: &str) -> bool;

    /// Resolves an identifier into one or more tracks.
    async fn load(
        &self,
        identifier: &str,
        routeplanner: Option<Arc<dyn RoutePlanner>>,
    ) -> LoadResult;

    /// Returns a playable track for the given identifier, if applicable.
    async fn get_track(
        &self,
        _identifier: &str,
        _routeplanner: Option<Arc<dyn RoutePlanner>>,
    ) -> Option<BoxedTrack> {
        None
    }

    /// Performs a search across various entities (tracks, albums, etc.).
    async fn load_search(
        &self,
        _query: &str,
        _types: &[String],
        _routeplanner: Option<Arc<dyn RoutePlanner>>,
    ) -> Option<SearchResult> {
        None
    }

    /// Returns the proxy configuration specific to this source.
    fn get_proxy_config(&self) -> Option<HttpProxyConfig> {
        None
    }

    /// Prefixes used for searching (e.g., "ytsearch:").
    fn search_prefixes(&self) -> Vec<&str> {
        vec![]
    }

    /// Prefixes used for ISRC lookups.
    fn isrc_prefixes(&self) -> Vec<&str> {
        vec![]
    }

    /// Prefixes used for recommendations.
    fn rec_prefixes(&self) -> Vec<&str> {
        vec![]
    }

    /// Indicates if this source acts as a mirror/resolver rather than a primary content provider.
    fn is_mirror(&self) -> bool {
        false
    }
}
