pub mod token;
pub mod metadata;
pub mod search;
pub mod utils;
pub mod track;

use std::sync::Arc;
use async_trait::async_trait;
use regex::Regex;
use crate::protocol::tracks::{LoadResult, SearchResult};
use crate::sources::{SourcePlugin, plugin::BoxedTrack};
use token::AmazonMusicTokenTracker;
use utils::extract_track_asin_param;
use metadata::fetch_track_duration_api;
use track::{fetch_json_ld, fallback_to_odesli};
use search::AmazonMusicSearch;

pub struct AmazonMusicSource {
    client: Arc<reqwest::Client>,
    token_tracker: Arc<AmazonMusicTokenTracker>,
    search_impl: AmazonMusicSearch,
    url_regex: Regex,
    search_prefixes: Vec<String>,
    search_limit: usize,
}

impl AmazonMusicSource {
    pub fn new(
        _config: Option<crate::configs::AmazonMusicConfig>,
        client: Arc<reqwest::Client>,
    ) -> Result<Self, String> {
        let token_tracker = Arc::new(AmazonMusicTokenTracker::new(client.clone()));
        let search_impl = AmazonMusicSearch::new(client.clone(), token_tracker.clone());

        let search_limit = _config.as_ref().map(|c| c.search_limit).unwrap_or(3).min(5);

        Ok(Self {
            client,
            token_tracker,
            search_impl,
            url_regex: Regex::new(r"https?://(?:music\.)?amazon\.[a-z.]+(?:/.*)?/(track|album|playlist|artist|dp)s?/([a-zA-Z0-9]+)").unwrap(),
            search_prefixes: vec![
                "amzsearch:".to_string(),
                "amazonsearch:".to_string(),
                "amznsearch:".to_string(),
                "amazonmusic:".to_string(),
            ],
            search_limit,
        })
    }
}

#[async_trait]
impl SourcePlugin for AmazonMusicSource {
    fn name(&self) -> &str {
        "amazonmusic"
    }

    fn can_handle(&self, identifier: &str) -> bool {
        self.search_prefixes.iter().any(|p| identifier.starts_with(p))
            || self.url_regex.is_match(identifier)
    }

    async fn load(
        &self,
        identifier: &str,
        _routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> LoadResult {
        if let Some(prefix) = self.search_prefixes.iter().find(|p| identifier.starts_with(*p)) {
            let query = &identifier[prefix.len()..];
            return self.search_impl.search(query, self.search_limit).await.map(|r| LoadResult::Search(r.tracks)).unwrap_or(LoadResult::Empty {});
        }

        if let Some(caps) = self.url_regex.captures(identifier) {
            let type_str = caps.get(1).map(|m: regex::Match| m.as_str()).unwrap_or("");
            let id = caps.get(2).map(|m: regex::Match| m.as_str()).unwrap_or("");
            if id.is_empty() { return LoadResult::Empty {}; }

            // Handle trackAsin param if present (highest priority)
            if let Some(track_asin) = extract_track_asin_param(identifier) {
                return self.resolve_track(identifier, &track_asin).await;
            }

            match type_str {
                "track" | "dp" => return self.resolve_track(identifier, id).await,
                "album" => return self.resolve_album(identifier, id).await,
                "playlist" => return self.resolve_playlist(identifier, id).await,
                "artist" => return self.resolve_artist(identifier, id).await,
                _ => {}
            }
        }

        LoadResult::Empty {}
    }

    async fn load_search(
        &self,
        query: &str,
        _types: &[String],
        _routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<SearchResult> {
        let mut q = query;
        for prefix in &self.search_prefixes {
            if q.starts_with(prefix) {
                q = &q[prefix.len()..];
                break;
            }
        }
        self.search_impl.search(q, self.search_limit).await
    }

    async fn get_track(
        &self,
        _identifier: &str,
        _routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<BoxedTrack> {
        None
    }

    fn search_prefixes(&self) -> Vec<&str> {
        self.search_prefixes.iter().map(|s| s.as_str()).collect()
    }

    fn is_mirror(&self) -> bool {
        true
    }
}

impl AmazonMusicSource {
    async fn resolve_track(&self, url: &str, id: &str) -> LoadResult {
        // Fallback to HTML scraping if API fails
        if let Some(mut result) = fetch_json_ld(&self.client, url, Some(id)).await {
            if let LoadResult::Track(ref mut t) = result {
                if t.info.length == 0 {
                    if let Some(duration) = fetch_track_duration_api(&self.client, &self.token_tracker, id, None).await {
                        t.info.length = duration;
                        t.encoded = t.encode();
                    }
                }
            }
            return result;
        }
        fallback_to_odesli(&self.client, url, Some(id)).await
    }

    async fn resolve_album(&self, url: &str, id: &str) -> LoadResult {
        if let Some(result) = fetch_json_ld(&self.client, url, None).await {
            return result;
        }
        fallback_to_odesli(&self.client, url, Some(id)).await
    }

    async fn resolve_playlist(&self, url: &str, id: &str) -> LoadResult {
        if let Some(result) = fetch_json_ld(&self.client, url, None).await {
            return result;
        }
        fallback_to_odesli(&self.client, url, Some(id)).await
    }

    async fn resolve_artist(&self, url: &str, id: &str) -> LoadResult {
        if let Some(result) = fetch_json_ld(&self.client, url, None).await {
            return result;
        }
        fallback_to_odesli(&self.client, url, Some(id)).await
    }
}
