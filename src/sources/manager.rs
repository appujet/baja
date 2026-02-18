use super::http::HttpSource;
use super::jiosaavn::JioSaavnSource;
use super::plugin::SourcePlugin;
use super::spotify::SpotifySource;
use super::youtube::YouTubeSource;
use std::sync::Arc;

/// Source Manager - coordinates all registered source plugins
pub struct SourceManager {
    sources: Vec<Box<dyn SourcePlugin>>,
}

impl SourceManager {
    /// Create a new SourceManager with all available sources
    pub fn new(config: &crate::config::Config) -> Self {
        let mut sources: Vec<Box<dyn SourcePlugin>> = Vec::new();

        // Register all sources in priority order
        // Specialized sources first
        if config.sources.jiosaavn {
            sources.push(Box::new(JioSaavnSource::new(config.jiosaavn.clone())));
        }
        if config.sources.youtube {
            sources.push(Box::new(YouTubeSource::new()));
        }
        if config.sources.spotify {
            sources.push(Box::new(SpotifySource::new()));
        }
        // Generic HTTP source last
        if config.sources.http {
            sources.push(Box::new(HttpSource::new()));
        }

        Self { sources }
    }

    /// Load tracks using the first matching source
    pub async fn load(
        &self,
        identifier: &str,
        routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> crate::api::tracks::LoadResult {
        // Try each source in order
        for source in &self.sources {
            if source.can_handle(identifier) {
                tracing::debug!("Loading '{}' with source: {}", identifier, source.name());
                return source.load(identifier, routeplanner.clone()).await;
            }
        }

        tracing::warn!("No source could handle identifier: {}", identifier);
        crate::api::tracks::LoadResult::Empty {}
    }

    pub async fn get_playback_url(
        &self,
        identifier: &str,
        routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<String> {
        for source in &self.sources {
            if source.can_handle(identifier) {
                tracing::debug!(
                    "Resolving playback URL for '{}' with source: {}",
                    identifier,
                    source.name()
                );
                return source
                    .get_playback_url(identifier, routeplanner.clone())
                    .await;
            }
        }
        // No source could handle it
        tracing::warn!("No source could resolve playback URL for: {}", identifier);
        None
    }

    /// Get names of all registered sources
    pub fn source_names(&self) -> Vec<String> {
        self.sources.iter().map(|s| s.name().to_string()).collect()
    }
}
