use std::sync::Arc;

use crate::{
    common::HttpClientPool,
    sources::plugin::{BoxedSource, BoxedTrack},
};

mod best_match;
mod registration;
mod resolver;

/// Source Manager handles the lifecycle and coordination of all audio sources.
pub struct SourceManager {
    pub sources: Vec<BoxedSource>,
    pub mirrors: Option<crate::config::server::MirrorsConfig>,
    pub youtube_cipher_manager: Option<Arc<crate::sources::youtube::cipher::YouTubeCipherManager>>,
    pub youtube_stream_ctx: Option<Arc<crate::sources::youtube::YoutubeStreamContext>>,
    pub http_pool: Arc<HttpClientPool>,
}

impl SourceManager {
    /// Create a new SourceManager with all available sources configured via AppConfig.
    pub fn new(config: &crate::config::AppConfig) -> Self {
        let http_pool = Arc::new(HttpClientPool::new());
        let mut sources = Vec::new();

        let (youtube_cipher_manager, youtube_stream_ctx) =
            registration::register_all(&mut sources, config, &http_pool);

        Self {
            sources,
            mirrors: config.player.mirrors.clone(),
            youtube_cipher_manager,
            youtube_stream_ctx,
            http_pool,
        }
    }

    /// Load tracks using the first matching source that can handle the identifier.
    pub async fn load(
        &self,
        identifier: &str,
        routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> crate::protocol::tracks::LoadResult {
        for source in &self.sources {
            if source.can_handle(identifier) {
                tracing::debug!(
                    "SourceManager: Loading '{}' with source: {}",
                    identifier,
                    source.name()
                );
                return source.load(identifier, routeplanner.clone()).await;
            }
        }

        tracing::debug!(
            "SourceManager: No source matched identifier: '{}'",
            identifier
        );
        crate::protocol::tracks::LoadResult::Empty {}
    }

    /// Perform a search across available sources.
    pub async fn load_search(
        &self,
        query: &str,
        types: &[String],
        routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<crate::protocol::tracks::SearchResult> {
        for source in &self.sources {
            if source.can_handle(query) {
                tracing::trace!("Loading search '{}' with source: {}", query, source.name());
                return source.load_search(query, types, routeplanner.clone()).await;
            }
        }

        tracing::debug!("No source could handle search query: {}", query);
        None
    }

    pub async fn resolve_track(
        &self,
        track_info: &crate::protocol::tracks::TrackInfo,
        routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Result<BoxedTrack, String> {
        let identifier = track_info.uri.as_deref().unwrap_or(&track_info.identifier);

        for source in &self.sources {
            if source.can_handle(identifier) {
                tracing::trace!(
                    "Resolving playable track for '{}' with source: {}",
                    identifier,
                    source.name()
                );

                if let Some(track) = source.get_track(identifier, routeplanner.clone()).await {
                    return Ok(track);
                }
                break;
            }
        }

        if let Some(mirrors) = &self.mirrors {
            return resolver::resolve_with_mirrors(
                self,
                track_info,
                identifier,
                mirrors,
                routeplanner,
            )
            .await;
        }

        Err(format!(
            "Failed to resolve playable track for: {}",
            identifier
        ))
    }

    /// Get names of all registered sources.
    pub fn source_names(&self) -> Vec<String> {
        self.sources.iter().map(|s| s.name().to_string()).collect()
    }

    /// Retrieves proxy configuration for a specific source by name.
    pub fn get_proxy_config(&self, source_name: &str) -> Option<crate::config::HttpProxyConfig> {
        self.sources
            .iter()
            .find(|s| s.name() == source_name)
            .and_then(|s| s.get_proxy_config())
    }
}
