use super::plugin::SourcePlugin;
use super::http::HttpSource;
use super::youtube::YouTubeSource;
use super::spotify::SpotifySource;

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
    pub async fn load(&self, identifier: &str) -> crate::api::tracks::LoadResult {
        // Try each source in order
        for source in &self.sources {
            if source.can_handle(identifier) {
                tracing::debug!("Loading '{}' with source: {}", identifier, source.name());
                return source.load(identifier).await;
            }
        }

        tracing::warn!("No source could handle identifier: {}", identifier);
        crate::api::tracks::LoadResult::Empty {}
    }

  
    pub async fn get_playback_url(&self, identifier: &str) -> Option<String> {
        for source in &self.sources {
            if source.can_handle(identifier) {
                tracing::debug!(
                    "Resolving playback URL for '{}' with source: {}",
                    identifier,
                    source.name()
                );
                return source.get_playback_url(identifier).await;
            }
        }
        // No source could handle it
        tracing::warn!("No source could resolve playback URL for: {}", identifier);
        None
    }
}
