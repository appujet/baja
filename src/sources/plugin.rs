use crate::rest::models::*;
use async_trait::async_trait;

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
    /// Returns a LoadTracksResponse with the appropriate load_type and data.
    async fn load(&self, identifier: &str) -> LoadTracksResponse;

    /// Get the actual playback URL for a given identifier.
    ///
    /// This is used to resolve search queries or platform URLs into direct audio streams.
    ///
    /// Examples:
    /// - HTTP source: returns the URL as-is
    /// - YouTube source: resolves "ytsearch:song name" or youtube.com URL to direct stream URL
    /// - Spotify source: resolves spotify.com URL to playable stream URL
    ///
    /// Returns None if the source cannot provide a playback URL.
    async fn get_playback_url(&self, identifier: &str) -> Option<String>;
}
