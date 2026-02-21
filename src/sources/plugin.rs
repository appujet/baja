use std::sync::Arc;

use async_trait::async_trait;
use flume::{Receiver, Sender};

use crate::audio::processor::DecoderCommand;

/// A track that can start its own decoding and return PCM samples.
pub trait PlayableTrack: Send + Sync {
    fn start_decoding(&self) -> (Receiver<i16>, Sender<DecoderCommand>);
}

/// A boxed playable track.
pub type BoxedTrack = Box<dyn PlayableTrack>;

/// A boxed source plugin.
pub type BoxedSource = Box<dyn SourcePlugin>;

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
    ) -> crate::api::tracks::LoadResult;

    /// Get a playable track for the given identifier.
    async fn get_track(
        &self,
        _identifier: &str,
        _routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<BoxedTrack> {
        None
    }

    /// Get the proxy configuration for this source, if any.
    fn get_proxy_config(&self) -> Option<crate::configs::HttpProxyConfig> {
        None
    }

    /// Get search prefixes for this source.
    fn search_prefixes(&self) -> Vec<&str> {
        Vec::new()
    }
}
