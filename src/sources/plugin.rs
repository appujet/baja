use async_trait::async_trait;
use std::sync::Arc;

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

    /// Get the actual playback URL for a given identifier.
    ///
    /// This is used to resolve search queries or platform URLs into direct audio streams.
    ///
    /// Returns None if the source cannot provide a playback URL.
    async fn get_playback_url(
        &self,
        identifier: &str,
        routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<String>;
}
