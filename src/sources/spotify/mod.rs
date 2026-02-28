use std::sync::Arc;

use async_trait::async_trait;
use regex::Regex;

use crate::{
    protocol::tracks::{LoadResult, Track},
    sources::{SourcePlugin, spotify::token::SpotifyTokenTracker},
};

pub mod helpers;
pub mod metadata;
pub mod parser;
pub mod recommendations;
pub mod search;
pub mod token;

pub struct SpotifySource {
    client: Arc<reqwest::Client>,
    search_prefixes: Vec<String>,
    rec_prefixes: Vec<String>,
    url_regex: Regex,
    mix_regex: Regex,
    isrc_binary_regex: Arc<Regex>,
    token_tracker: Arc<SpotifyTokenTracker>,

    playlist_load_limit: usize,
    album_load_limit: usize,
    search_limit: usize,
    recommendations_limit: usize,

    playlist_page_load_concurrency: usize,
    album_page_load_concurrency: usize,
    track_resolve_concurrency: usize,
}

impl SpotifySource {
    pub fn new(
        config: Option<crate::configs::SpotifyConfig>,
        client: Arc<reqwest::Client>,
    ) -> Result<Self, String> {
        let (
            playlist_load_limit,
            album_load_limit,
            search_limit,
            recommendations_limit,
            playlist_page_load_concurrency,
            album_page_load_concurrency,
            track_resolve_concurrency,
        ) = if let Some(c) = config {
            (
                c.playlist_load_limit,
                c.album_load_limit,
                c.search_limit,
                c.recommendations_limit,
                c.playlist_page_load_concurrency,
                c.album_page_load_concurrency,
                c.track_resolve_concurrency,
            )
        } else {
            (6, 6, 10, 10, 10, 5, 50)
        };

        let token_tracker = Arc::new(SpotifyTokenTracker::new(client.clone()));
        token_tracker.clone().init();

        Ok(Self {
      client,
      search_prefixes: vec!["spsearch:".to_string()],
      rec_prefixes: vec!["sprec:".to_string()],
      url_regex: Regex::new(
        r"https?://(?:open\.)?spotify\.com/(?:intl-[a-z]{2}/)?(track|album|playlist|artist)/([a-zA-Z0-9]+)",
      ).unwrap(),
      mix_regex: Regex::new(r"mix:(album|artist|track|isrc):([a-zA-Z0-9\-_]+)").unwrap(),
      isrc_binary_regex: Arc::new(Regex::new(r"[A-Z0-9]{12}").unwrap()),
      token_tracker,
      playlist_load_limit,
      album_load_limit,
      search_limit,
      recommendations_limit,
      playlist_page_load_concurrency,
      album_page_load_concurrency,
      track_resolve_concurrency,
    })
    }

    pub async fn get_autocomplete(
        &self,
        query: &str,
        types: &[String],
    ) -> Option<crate::protocol::tracks::SearchResult> {
        search::SpotifySearch::get_autocomplete(
            &self.client,
            &self.token_tracker,
            query,
            types,
            self.search_limit,
            &self.isrc_binary_regex,
        )
        .await
    }

    pub fn base_request(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        builder.header(reqwest::header::USER_AGENT, "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.6998.178 Spotify/1.2.65.255 Safari/537.36")
    }
}

#[async_trait]
impl SourcePlugin for SpotifySource {
    fn name(&self) -> &str {
        "spotify"
    }

    fn can_handle(&self, identifier: &str) -> bool {
        self.search_prefixes
            .iter()
            .any(|p| identifier.starts_with(p))
            || self.rec_prefixes.iter().any(|p| identifier.starts_with(p))
            || self.url_regex.is_match(identifier)
    }

    fn search_prefixes(&self) -> Vec<&str> {
        self.search_prefixes.iter().map(|s| s.as_str()).collect()
    }

    fn is_mirror(&self) -> bool {
        true
    }

    async fn load(
        &self,
        identifier: &str,
        _routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> LoadResult {
        if let Some(prefix) = self
            .search_prefixes
            .iter()
            .find(|p| identifier.starts_with(*p))
        {
            let query = &identifier[prefix.len()..];
            return match self.get_autocomplete(query, &["track".to_string()]).await {
                Some(res) => {
                    if res.tracks.is_empty() {
                        LoadResult::Empty {}
                    } else {
                        LoadResult::Search(res.tracks)
                    }
                }
                None => LoadResult::Empty {},
            };
        }

        if let Some(prefix) = self
            .rec_prefixes
            .iter()
            .find(|p| identifier.starts_with(*p))
        {
            let query = &identifier[prefix.len()..];
            return match recommendations::SpotifyRecommendations::fetch_recommendations(
                &self.client,
                &self.token_tracker,
                query,
                &self.mix_regex,
                self.recommendations_limit,
                self.search_limit,
                &self.isrc_binary_regex,
            )
            .await
            {
                Ok(res) => res,
                Err(playlist_id) => {
                    metadata::SpotifyMetadata::fetch_playlist(
                        &self.client,
                        &self.token_tracker,
                        &playlist_id,
                        self.playlist_load_limit,
                        self.playlist_page_load_concurrency,
                        self.track_resolve_concurrency,
                        &self.isrc_binary_regex,
                    )
                    .await
                }
            };
        }

        if let Some(caps) = self.url_regex.captures(identifier) {
            let type_str = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            let id = caps.get(2).map(|m| m.as_str()).unwrap_or("");

            match type_str {
                "track" => {
                    if let Some(track_info) = metadata::SpotifyMetadata::fetch_track(
                        &self.client,
                        &self.token_tracker,
                        id,
                        &self.isrc_binary_regex,
                    )
                    .await
                    {
                        return LoadResult::Track(Track::new(track_info));
                    }
                }
                "album" => {
                    return metadata::SpotifyMetadata::fetch_album(
                        &self.client,
                        &self.token_tracker,
                        id,
                        self.album_load_limit,
                        self.album_page_load_concurrency,
                        self.track_resolve_concurrency,
                        &self.isrc_binary_regex,
                    )
                    .await;
                }
                "playlist" => {
                    return metadata::SpotifyMetadata::fetch_playlist(
                        &self.client,
                        &self.token_tracker,
                        id,
                        self.playlist_load_limit,
                        self.playlist_page_load_concurrency,
                        self.track_resolve_concurrency,
                        &self.isrc_binary_regex,
                    )
                    .await;
                }
                "artist" => {
                    return metadata::SpotifyMetadata::fetch_artist(
                        &self.client,
                        &self.token_tracker,
                        id,
                        &self.isrc_binary_regex,
                    )
                    .await;
                }
                _ => {}
            }
        }

        LoadResult::Empty {}
    }

    async fn load_search(
        &self,
        query: &str,
        types: &[String],
        _routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<crate::protocol::tracks::SearchResult> {
        let mut q = query;
        for prefix in &self.search_prefixes {
            if q.starts_with(prefix) {
                q = &q[prefix.len()..];
                break;
            }
        }
        self.get_autocomplete(q, types).await
    }

    async fn get_track(
        &self,
        _identifier: &str,
        _routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<crate::sources::plugin::BoxedTrack> {
        None
    }
}
