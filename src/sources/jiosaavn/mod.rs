use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use regex::Regex;

use self::track::JioSaavnTrack;
use crate::{protocol::tracks::LoadResult, sources::plugin::PlayableTrack};

pub mod helpers;
pub mod metadata;
pub mod parser;
pub mod reader;
pub mod recommendations;
pub mod search;
pub mod track;

fn url_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"https?://(?:www\.)?(?:jiosaavn|saavn)\.com/(?:(?<type>p/album|s/featured|s/artist|s/song|album|featured|song|s/playlist|artist)/)(?:[^/]+/)+(?<id>[A-Za-z0-9_,-]+)").unwrap()
    })
}

pub struct JioSaavnSource {
    pub(crate) client: Arc<reqwest::Client>,
    pub(crate) secret_key: Vec<u8>,
    pub(crate) proxy: Option<crate::config::HttpProxyConfig>,
    // Limits
    pub(crate) search_limit: usize,
    pub(crate) recommendations_limit: usize,
    pub(crate) playlist_load_limit: usize,
    pub(crate) album_load_limit: usize,
    pub(crate) artist_load_limit: usize,
}

impl JioSaavnSource {
    pub fn new(
        config: Option<crate::config::JioSaavnConfig>,
        client: Arc<reqwest::Client>,
    ) -> Result<Self, String> {
        let (
            secret_key,
            search_limit,
            recommendations_limit,
            playlist_load_limit,
            album_load_limit,
            artist_load_limit,
            proxy,
        ) = if let Some(c) = config {
            (
                c.decryption
                    .and_then(|d| d.secret_key)
                    .unwrap_or_else(|| "38346591".to_owned()),
                c.search_limit,
                c.recommendations_limit,
                c.playlist_load_limit,
                c.album_load_limit,
                c.artist_load_limit,
                c.proxy,
            )
        } else {
            ("38346591".to_owned(), 10, 10, 50, 50, 20, None)
        };

        Ok(Self {
            client,
            secret_key: secret_key.into_bytes(),
            proxy,
            search_limit,
            recommendations_limit,
            playlist_load_limit,
            album_load_limit,
            artist_load_limit,
        })
    }
}

#[async_trait]
impl crate::sources::plugin::SourcePlugin for JioSaavnSource {
    fn name(&self) -> &str {
        "jiosaavn"
    }

    fn can_handle(&self, identifier: &str) -> bool {
        self.search_prefixes()
            .iter()
            .any(|p| identifier.starts_with(p))
            || self
                .rec_prefixes()
                .iter()
                .any(|p| identifier.starts_with(p))
            || url_regex().is_match(identifier)
    }

    fn search_prefixes(&self) -> Vec<&str> {
        vec!["jssearch:"]
    }

    fn rec_prefixes(&self) -> Vec<&str> {
        vec!["jsrec:"]
    }

    async fn load(
        &self,
        identifier: &str,
        _routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> LoadResult {
        for prefix in self.rec_prefixes() {
            if let Some(query) = identifier.strip_prefix(prefix) {
                return self.get_recommendations(query).await;
            }
        }

        for prefix in self.search_prefixes() {
            if let Some(query) = identifier.strip_prefix(prefix) {
                return self.search(query).await;
            }
        }

        if let Some(caps) = url_regex().captures(identifier) {
            let type_ = caps.name("type").map(|m| m.as_str()).unwrap_or("");
            let id = caps.name("id").map(|m| m.as_str()).unwrap_or("");

            if id.is_empty() || type_.is_empty() {
                return LoadResult::Empty {};
            }

            let canonical_type = match type_ {
                "p/album" => "album",
                "s/artist" => "artist",
                "s/featured" => "featured",
                other => other,
            };

            if canonical_type == "song" || canonical_type == "s/song" {
                if let Some(track_data) = self.fetch_metadata(id).await
                    && let Some(track) = parser::parse_track(&track_data)
                {
                    return LoadResult::Track(track);
                }
                return LoadResult::Empty {};
            } else {
                return self.resolve_list(canonical_type, id).await;
            }
        }

        LoadResult::Empty {}
    }

    async fn get_track(
        &self,
        identifier: &str,
        routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<Box<dyn PlayableTrack>> {
        let id = if let Some(caps) = url_regex().captures(identifier) {
            caps.name("id").map(|m| m.as_str()).unwrap_or(identifier)
        } else {
            identifier
        };

        let track_data = self.fetch_metadata(id).await?;
        let encrypted_url = track_data
            .get("more_info")
            .and_then(|m| m.get("encrypted_media_url"))
            .and_then(|v| v.as_str())?
            .to_owned();

        let is_320 = track_data
            .get("more_info")
            .and_then(|m| m.get("320kbps"))
            .map(|v| v.as_str() == Some("true") || v.as_bool() == Some(true))
            .unwrap_or(false);

        let local_addr = routeplanner.and_then(|rp| rp.get_address());

        Some(Box::new(JioSaavnTrack {
            encrypted_url,
            secret_key: self.secret_key.clone(),
            is_320,
            local_addr,
            proxy: self.proxy.clone(),
        }))
    }

    fn get_proxy_config(&self) -> Option<crate::config::HttpProxyConfig> {
        self.proxy.clone()
    }

    async fn load_search(
        &self,
        query: &str,
        types: &[String],
        _routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<crate::protocol::tracks::SearchResult> {
        let mut q = query;
        for prefix in self.search_prefixes() {
            if let Some(stripped) = query.strip_prefix(prefix) {
                q = stripped;
                break;
            }
        }
        self.get_autocomplete(q, types).await
    }
}
