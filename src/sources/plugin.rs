use std::sync::Arc;

use async_trait::async_trait;
use flume::{Receiver, Sender};

use crate::audio::processor::DecoderCommand;

/// A track that can start its own decoding and return PCM samples.
/// Returns `(pcm_rx, cmd_tx, error_rx)` where:
/// - `pcm_rx`   — batched i16 PCM sample frames from the decoder
/// - `cmd_tx`   — send seek/stop commands to the decoder
/// - `error_rx` — receives a single `String` if a fatal decode/IO error occurs
pub trait PlayableTrack: Send + Sync {
  fn start_decoding(
    &self,
  ) -> (
    Receiver<Vec<i16>>,
    Sender<DecoderCommand>,
    flume::Receiver<String>,
  );
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

  fn search_prefixes(&self) -> Vec<&str> {
    vec![]
  }

  fn rec_prefixes(&self) -> Vec<&str> {
    vec![]
  }
}
