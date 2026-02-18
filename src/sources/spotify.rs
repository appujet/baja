use crate::api::tracks::LoadResult;
use crate::sources::SourcePlugin;
use async_trait::async_trait;
use regex::Regex;
use std::sync::Arc;

/// Spotify Source Plugin (Placeholder)
///
/// Will handle:
/// - spsearch: prefix for searches
/// - spotify.com URLs (tracks, albums, playlists)
///
/// TODO: Integrate Spotify API for actual resolution
pub struct SpotifySource {
    search_prefix: String,
    url_regex: Regex,
}

impl SpotifySource {
    pub fn new() -> Self {
        Self {
            search_prefix: "spsearch:".to_string(),
            // Matches spotify.com URLs
            url_regex: Regex::new(r"spotify\.com/(track|album|playlist)").unwrap(),
        }
    }
}

#[async_trait]
impl SourcePlugin for SpotifySource {
    fn name(&self) -> &str {
        "spotify"
    }

    fn can_handle(&self, identifier: &str) -> bool {
        identifier.starts_with(&self.search_prefix) || self.url_regex.is_match(identifier)
    }

    async fn load(
        &self,
        _identifier: &str,
        _routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> LoadResult {
        // TODO: Implement actual Spotify API integration
        // For now, return empty
        LoadResult::Empty {}
    }

    async fn get_playback_url(
        &self,
        identifier: &str,
        _routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<String> {
        // TODO: Implement Spotify URL resolution
        // This should:
        // 1. For "spsearch:query" - search Spotify and get the first result
        // 2. For spotify.com URLs - extract track/album/playlist ID
        // 3. Use Spotify API to get track info, then resolve to playable stream
        //    (Note: Spotify doesn't provide direct streams, may need to use a proxy service)

        // For now, return None (not implemented)
        tracing::warn!(
            "Spotify playback URL resolution not yet implemented for: {}",
            identifier
        );
        None
    }
}
