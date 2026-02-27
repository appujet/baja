use std::sync::Arc;

use async_trait::async_trait;
use regex::Regex;
use reqwest::header::HeaderMap;
use tracing::debug;

use self::track::JioSaavnTrack;
use crate::{protocol::tracks::LoadResult, sources::plugin::PlayableTrack};

pub mod helpers;
pub mod metadata;
pub mod parser;
pub mod reader;
pub mod recommendations;
pub mod search;
pub mod track;

pub struct JioSaavnSource {
    pub(crate) client: reqwest::Client,
    pub(crate) url_regex: Regex,
    pub(crate) search_prefixes: Vec<String>,
    pub(crate) rec_prefixes: Vec<String>,
    pub(crate) secret_key: Vec<u8>,
    pub(crate) proxy: Option<crate::configs::HttpProxyConfig>,
    // Limits
    pub(crate) search_limit: usize,
    pub(crate) recommendations_limit: usize,
    pub(crate) playlist_load_limit: usize,
    pub(crate) album_load_limit: usize,
    pub(crate) artist_load_limit: usize,
}

impl JioSaavnSource {
    pub fn new(config: Option<crate::configs::JioSaavnConfig>) -> Result<Self, String> {
        let mut headers = HeaderMap::new();

        headers.insert(
      "User-Agent",
      "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/115.0.0.0 Safari/537.36"
        .parse()
        .unwrap(),
    );

        headers.insert("Accept", "application/json".parse().unwrap());
        headers.insert("Accept-Language", "en-US,en;q=0.9".parse().unwrap());
        headers.insert("Referer", "https://www.jiosaavn.com/".parse().unwrap());
        headers.insert("Origin", "https://www.jiosaavn.com".parse().unwrap());

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
                    .unwrap_or_else(|| "38346591".to_string()),
                c.search_limit,
                c.recommendations_limit,
                c.playlist_load_limit,
                c.album_load_limit,
                c.artist_load_limit,
                c.proxy,
            )
        } else {
            ("38346591".to_string(), 10, 10, 50, 50, 20, None)
        };

        let mut client_builder = reqwest::Client::builder().default_headers(headers);

        if let Some(proxy_config) = &proxy {
            if let Some(url) = &proxy_config.url {
                debug!("Configuring proxy for JioSaavnSource: {}", url);
                if let Ok(proxy_obj) = reqwest::Proxy::all(url) {
                    let mut proxy_obj = proxy_obj;
                    if let (Some(username), Some(password)) =
                        (&proxy_config.username, &proxy_config.password)
                    {
                        proxy_obj = proxy_obj.basic_auth(username, password);
                    }
                    client_builder = client_builder.proxy(proxy_obj);
                }
            }
        }

        let client = client_builder.build().map_err(|e| e.to_string())?;

        Ok(Self {
      client,
      url_regex: Regex::new(r"https?://(?:www\.)?jiosaavn\.com/(?:(?<type>album|featured|song|s/playlist|artist)/)(?:[^/]+/)(?<id>[A-Za-z0-9_,-]+)").unwrap(),
      search_prefixes: vec!["jssearch:".to_string()],
      rec_prefixes: vec!["jsrec:".to_string()],
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
        self.search_prefixes
            .iter()
            .any(|p| identifier.starts_with(p))
            || self.rec_prefixes.iter().any(|p| identifier.starts_with(p))
            || self.url_regex.is_match(identifier)
    }

    fn search_prefixes(&self) -> Vec<&str> {
        self.search_prefixes.iter().map(|s| s.as_str()).collect()
    }

    fn rec_prefixes(&self) -> Vec<&str> {
        self.rec_prefixes.iter().map(|s| s.as_str()).collect()
    }

    async fn load(
        &self,
        identifier: &str,
        _routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> LoadResult {
        if let Some(prefix) = self
            .rec_prefixes
            .iter()
            .find(|p| identifier.starts_with(*p))
        {
            let query = identifier.strip_prefix(prefix).unwrap();
            return self.get_recommendations(query).await;
        }

        if let Some(prefix) = self
            .search_prefixes
            .iter()
            .find(|p| identifier.starts_with(*p))
        {
            let query = identifier.strip_prefix(prefix).unwrap();
            return self.search(query).await;
        }

        // Regex Match URL
        if let Some(caps) = self.url_regex.captures(identifier) {
            let type_ = caps.name("type").map(|m| m.as_str()).unwrap_or("");
            let id = caps.name("id").map(|m| m.as_str()).unwrap_or("");

            if id.is_empty() || type_.is_empty() {
                return LoadResult::Empty {};
            }

            if type_ == "song" {
                if let Some(track_data) = self.fetch_metadata(id).await {
                    if let Some(track) = parser::parse_track(&track_data) {
                        return LoadResult::Track(track);
                    }
                }
                return LoadResult::Empty {};
            } else {
                return self.resolve_list(type_, id).await;
            }
        }

        LoadResult::Empty {}
    }

    async fn get_track(
        &self,
        identifier: &str,
        routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<Box<dyn PlayableTrack>> {
        let id = if let Some(caps) = self.url_regex.captures(identifier) {
            caps.name("id").map(|m| m.as_str()).unwrap_or(identifier)
        } else {
            identifier
        };

        let track_data = self.fetch_metadata(id).await?;
        let encrypted_url = track_data
            .get("more_info")
            .and_then(|m| m.get("encrypted_media_url"))
            .and_then(|v| v.as_str())?
            .to_string();

        let is_320 = track_data
            .get("more_info")
            .and_then(|m| m.get("320kbps"))
            .map(|v| v.as_str() == Some("true") || v.as_bool() == Some(true))
            .unwrap_or(false);

        let local_addr = if let Some(rp) = routeplanner {
            rp.get_address()
        } else {
            None
        };

        Some(Box::new(JioSaavnTrack {
            encrypted_url,
            secret_key: self.secret_key.clone(),
            is_320,
            local_addr,
            proxy: self.proxy.clone(),
        }))
    }

    fn get_proxy_config(&self) -> Option<crate::configs::HttpProxyConfig> {
        self.proxy.clone()
    }

    async fn load_search(
        &self,
        query: &str,
        types: &[String],
        _routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<crate::protocol::tracks::SearchResult> {
        let q = if let Some(prefix) = self.search_prefixes.iter().find(|p| query.starts_with(*p)) {
            query.strip_prefix(prefix).unwrap()
        } else {
            query
        };

        self.get_autocomplete(q, types).await
    }
}
