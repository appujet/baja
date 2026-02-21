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

  /// Search across various entities (tracks, albums, artists, etc).
  /// Corresponds to LavaSearch's loadSearch API.
  async fn load_search(
    &self,
    _query: &str,
    _types: &[String],
    _routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
  ) -> Option<crate::api::tracks::SearchResult> {
    None
  }

  /// Get the proxy configuration for this source, if any.
  fn get_proxy_config(&self) -> Option<crate::configs::HttpProxyConfig> {
    None
  }
}
