use super::http::HttpSource;
use super::jiosaavn::JioSaavnSource;
use super::plugin::SourcePlugin;
use super::spotify::SpotifySource;
use super::youtube::YouTubeSource;
use std::sync::Arc;

/// Source Manager - coordinates all registered source plugins
pub struct SourceManager {
    sources: Vec<Box<dyn SourcePlugin>>,
    mirrors: Option<crate::configs::MirrorsConfig>,
}

impl SourceManager {
    /// Create a new SourceManager with all available sources
    pub fn new(config: &crate::configs::Config) -> Self {
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
            sources.push(Box::new(SpotifySource::new(config.spotify.clone())));
        }
        // Generic HTTP source last
        if config.sources.http {
            sources.push(Box::new(HttpSource::new()));
        }

        Self {
            sources,
            mirrors: config.mirrors.clone(),
        }
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
        track_info: &crate::api::tracks::TrackInfo,
        routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<String> {
        let identifier = track_info.uri.as_deref().unwrap_or(&track_info.identifier);

        // 1. Try resolving with the original source first
        for source in &self.sources {
            if source.can_handle(identifier) {
                tracing::debug!(
                    "Resolving playback URL for '{}' with source: {}",
                    identifier,
                    source.name()
                );

                if let Some(url) = source
                    .get_playback_url(identifier, routeplanner.clone())
                    .await
                {
                    return Some(url);
                }

                // If this source claimed to handle it but returned None,
                // AND it's the one that should have handled it (we assume first match is correct source),
                // then we fall through to mirrors.
                // We break the loop so we don't try other sources for direct resolution.
                break;
            }
        }

        // 2. If direct resolution failed (or no source handled it), try mirrors
        if let Some(mirrors) = &self.mirrors {
            let isrc = track_info.isrc.as_deref().unwrap_or("");
            let query = format!("{} - {}", track_info.title, track_info.author);

            if isrc.is_empty() {
                tracing::debug!("Track has no ISRC");
            }

            for provider in &mirrors.providers {
                let search_query = provider.replace("%ISRC%", isrc).replace("%QUERY%", &query);

                // Skip if ISRC is empty but provider requires it
                if isrc.is_empty() && provider.contains("%ISRC%") {
                    continue;
                }

                tracing::debug!("Attempting mirror provider: {}", search_query);

                // Use the manager's own load() to resolve the mirror query (e.g. "ytsearch:Title - Artist")
                match self.load(&search_query, routeplanner.clone()).await {
                    crate::api::tracks::LoadResult::Track(track) => {
                        let nested_id = track.info.uri.as_deref().unwrap_or(&track.info.identifier);
                        if let Some(url) = self
                            .resolve_nested_id(nested_id, routeplanner.clone())
                            .await
                        {
                            tracing::debug!("Mirror success: {} -> {}", search_query, url);
                            return Some(url);
                        }
                    }
                    crate::api::tracks::LoadResult::Search(tracks) => {
                        if let Some(first_track) = tracks.first() {
                            let nested_id = first_track
                                .info
                                .uri
                                .as_deref()
                                .unwrap_or(&first_track.info.identifier);
                            if let Some(url) = self
                                .resolve_nested_id(nested_id, routeplanner.clone())
                                .await
                            {
                                tracing::debug!(
                                    "Mirror success (search): {} -> {}",
                                    search_query,
                                    url
                                );
                                return Some(url);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        tracing::warn!("Failed to resolve playback URL for: {}", identifier);
        None
    }

    /// Helper to resolve a nested ID found via mirror search
    async fn resolve_nested_id(
        &self,
        identifier: &str,
        routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<String> {
        for source in &self.sources {
            if source.can_handle(identifier) {
                if let Some(url) = source
                    .get_playback_url(identifier, routeplanner.clone())
                    .await
                {
                    return Some(url);
                }
            }
        }
        None
    }

    /// Get names of all registered sources
    pub fn source_names(&self) -> Vec<String> {
        self.sources.iter().map(|s| s.name().to_string()).collect()
    }
}
