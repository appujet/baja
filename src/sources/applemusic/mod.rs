pub mod helpers;
pub mod metadata;
pub mod parser;
pub mod search;
pub mod token;

use std::sync::Arc;

use async_trait::async_trait;
use regex::Regex;
use reqwest::header::{HeaderMap, HeaderValue, USER_AGENT};
use token::AppleMusicTokenTracker;

use crate::{api::tracks::LoadResult, sources::SourcePlugin};

const API_BASE: &str = "https://api.music.apple.com/v1";

pub struct AppleMusicSource {
    client: reqwest::Client,
    token_tracker: Arc<AppleMusicTokenTracker>,
    country_code: String,

    playlist_load_limit: usize,
    album_load_limit: usize,
    playlist_page_load_concurrency: usize,
    album_page_load_concurrency: usize,
    search_prefixes: Vec<String>,
    url_regex: Regex,
}

impl AppleMusicSource {
    pub fn new(config: Option<crate::configs::AppleMusicConfig>) -> Result<Self, String> {
        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, HeaderValue::from_static("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.0.0 Safari/537.36"));

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .gzip(true)
            .build()
            .map_err(|e| e.to_string())?;

        let (country, p_limit, a_limit, p_conc, a_conc) = if let Some(c) = config {
            (
                c.country_code,
                c.playlist_load_limit,
                c.album_load_limit,
                c.playlist_page_load_concurrency,
                c.album_page_load_concurrency,
            )
        } else {
            ("us".to_string(), 0, 0, 5, 5)
        };

        let token_tracker = Arc::new(
            crate::sources::applemusic::token::AppleMusicTokenTracker::new(client.clone()),
        );
        token_tracker.clone().init();

        Ok(Self {
      token_tracker,
      client,
      country_code: country,
      playlist_load_limit: p_limit,
      album_load_limit: a_limit,
      playlist_page_load_concurrency: p_conc,
      album_page_load_concurrency: a_conc,
      search_prefixes: vec!["amsearch:".to_string()],
      url_regex: Regex::new(r"https?://(?:www\.)?music\.apple\.com/(?:[a-zA-Z]{2}/)?(album|playlist|artist|song)/[^/]+/([a-zA-Z0-9\-.]+)(?:\?i=(\d+))?").unwrap(),
    })
    }
}

#[async_trait]
impl SourcePlugin for AppleMusicSource {
    fn name(&self) -> &str {
        "applemusic"
    }

    fn can_handle(&self, identifier: &str) -> bool {
        self.search_prefixes
            .iter()
            .any(|p| identifier.starts_with(p))
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
            return self.search(query).await;
        }

        if let Some(caps) = self.url_regex.captures(identifier) {
            let type_str = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            let id = caps.get(2).map(|m| m.as_str()).unwrap_or("");
            let song_id = caps.get(3).map(|m| m.as_str());

            if type_str == "album" && song_id.is_some() {
                return self.resolve_track(song_id.unwrap()).await;
            }

            match type_str {
                "song" => return self.resolve_track(id).await,
                "album" => return self.resolve_album(id).await,
                "playlist" => return self.resolve_playlist(id).await,
                "artist" => return self.resolve_artist(id).await,
                _ => return LoadResult::Empty {},
            }
        }

        LoadResult::Empty {}
    }

    async fn load_search(
        &self,
        query: &str,
        types: &[String],
        _routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<crate::api::tracks::SearchResult> {
        let q = if let Some(prefix) = self.search_prefixes.iter().find(|p| query.starts_with(*p)) {
            &query[prefix.len()..]
        } else {
            query
        };

        self.get_search_suggestions(q, types).await
    }

    async fn get_track(
        &self,
        _identifier: &str,
        _routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<crate::sources::plugin::BoxedTrack> {
        None
    }
}
