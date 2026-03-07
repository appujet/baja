pub mod helpers;
pub mod metadata;
pub mod search;

use std::sync::{Arc, OnceLock};
use async_trait::async_trait;
use regex::Regex;

use crate::{
    protocol::tracks::LoadResult,
    sources::plugin::{BoxedTrack, SourcePlugin},
};

pub fn path_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"https?://(?:www\.)?last\.fm/(?:[a-z]{2}/)?(music|user)/([^/]+)(?:/([^/]+)(?:/([^/]+))?)?")
            .expect("lastfm path regex is a valid literal")
    })
}

pub fn search_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r#"(?i)<tr[^>]*?>[\s\S]*?<img[^>]*?src="([^"]+)"[\s\S]*?data-track-name="([^"]+)"[\s\S]*?data-track-url="([^"]+)"[\s\S]*?data-artist-name="([^"]+)"#)
            .expect("lastfm search regex is a valid literal")
    })
}

pub fn encode_path_segment(segment: &str) -> String {
    urlencoding::encode(segment).replace("%20", "+")
}

pub fn construct_track_url(artist: &str, track: &str) -> String {
    format!(
        "https://www.last.fm/music/{}/_/{}",
        encode_path_segment(artist),
        encode_path_segment(track)
    )
}

pub struct LastFMSource {
    pub http: Arc<reqwest::Client>,
    pub api_key: Option<String>,
    pub search_limit: usize,
}

impl LastFMSource {
    pub fn new(
        config: Option<crate::config::sources::LastFmConfig>,
        http: Arc<reqwest::Client>,
    ) -> Result<Self, String> {
        let (api_key, search_limit) = if let Some(c) = config {
            (c.api_key, c.search_limit)
        } else {
            (None, 10)
        };

        Ok(Self {
            http,
            api_key,
            search_limit,
        })
    }
}

#[async_trait]
impl SourcePlugin for LastFMSource {
    fn name(&self) -> &str {
        "lastfm"
    }

    fn search_prefixes(&self) -> Vec<&str> {
        vec!["lfsearch:", "lfmsearch:"]
    }

    fn can_handle(&self, identifier: &str) -> bool {
        self.search_prefixes()
            .iter()
            .any(|p| identifier.starts_with(p))
            || path_regex().is_match(identifier)
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
            .search_prefixes()
            .into_iter()
            .find(|p| identifier.starts_with(p))
        {
            let query = &identifier[prefix.len()..];
            self.search_tracks(query).await
        } else {
            self.resolve_url(identifier).await
        }
    }

    async fn get_track(
        &self,
        _identifier: &str,
        _routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<BoxedTrack> {
        None
    }
}
