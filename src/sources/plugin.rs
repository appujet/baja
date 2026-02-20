use async_trait::async_trait;
use flume::{Receiver, Sender};
use std::sync::Arc;

use crate::audio::processor::DecoderCommand;

/// A track that can start its own decoding and return PCM samples.
pub trait PlayableTrack: Send + Sync {
    fn start_decoding(&self) -> (Receiver<i16>, Sender<DecoderCommand>);
}

/// Trait that all source plugins must implement.
///
/// Each source (HTTP, YouTube, Spotify, etc.) implements this trait
/// to provide track resolution capabilities.
#[async_trait]
pub trait SourcePlugin: Send + Sync {
    /// Unique identifier for this source (e.g., "http", "youtube", "spotify")
    fn name(&self) -> &str;

    /// Check if this source can handle the given identifier.
    ///
    /// Examples:
    /// - HTTP source: checks for http:// or https://
    /// - YouTube source: checks for ytsearch: prefix or youtube.com URLs
    /// - Spotify source: checks for spsearch: prefix or spotify.com URLs
    fn can_handle(&self, identifier: &str) -> bool;

    /// Resolve the identifier into track(s).
    ///
    /// Returns a LoadResult with the appropriate load_type and data.
    async fn load(
        &self,
        identifier: &str,
        routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> crate::api::tracks::LoadResult;

    /// Get a playable track for the given identifier.
    ///
    /// This is the new pattern replacing get_playback_url.
    async fn get_track(
        &self,
        _identifier: &str,
        _routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<Box<dyn PlayableTrack>> {
        None
    }

    /// Get the proxy configuration for this source, if any.
    fn get_proxy_config(&self) -> Option<crate::configs::HttpProxyConfig> {
        None
    }
}
