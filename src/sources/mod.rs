use crate::rest::models::*;
use async_trait::async_trait;

pub mod http;
pub mod spotify;
pub mod youtube;

pub use http::HttpSource;
pub use spotify::SpotifySource;
pub use youtube::YouTubeSource;

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

/// Source Manager - coordinates all registered source plugins
pub struct SourceManager {
    sources: Vec<Box<dyn SourcePlugin>>,
}

impl SourceManager {
    /// Create a new SourceManager with all available sources
    pub fn new() -> Self {
        let mut sources: Vec<Box<dyn SourcePlugin>> = Vec::new();

        // Register all sources in priority order
        sources.push(Box::new(HttpSource::new()));
        sources.push(Box::new(YouTubeSource::new()));
        sources.push(Box::new(SpotifySource::new()));

        Self { sources }
    }

    /// Load tracks using the first matching source
    pub async fn load(&self, identifier: &str) -> LoadTracksResponse {
        // Try each source in order
        for source in &self.sources {
            if source.can_handle(identifier) {
                tracing::info!("Loading '{}' with source: {}", identifier, source.name());
                return source.load(identifier).await;
            }
        }

        // No source could handle it
        tracing::warn!("No source could handle identifier: {}", identifier);
        LoadTracksResponse {
            load_type: LoadType::Empty,
            data: LoadData::Empty(serde_json::Value::Null),
        }
    }

    /// Get the actual playback URL for an identifier
    ///
    /// This resolves search queries or platform URLs into direct audio stream URLs.
    /// Used by the player to get the actual URL to stream from.
    pub async fn get_playback_url(&self, identifier: &str) -> Option<String> {
        // Clean the identifier
        let clean = identifier
            .trim()
            .trim_start_matches('<')
            .trim_end_matches('>');

        // Try each source in order
        for source in &self.sources {
            if source.can_handle(clean) {
                tracing::info!(
                    "Resolving playback URL for '{}' with source: {}",
                    clean,
                    source.name()
                );
                return source.get_playback_url(clean).await;
            }
        }

        // No source could handle it
        tracing::warn!("No source could resolve playback URL for: {}", clean);
        None
    }
}
