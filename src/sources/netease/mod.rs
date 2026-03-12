use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use rand::Rng;
use regex::Regex;
use tracing::debug;

use crate::{
    protocol::tracks::{LoadResult, SearchResult},
    sources::{SourcePlugin, plugin::BoxedTrack},
};

pub mod api;
pub mod manager;
pub mod track;

fn url_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"https?://music\.163\.com/(?:(?:#|m)/)?(?P<type>song|album|playlist|artist)(?:\?id=|\/)(?P<id>\d+)").unwrap()
    })
}

pub struct NeteaseSource {
    pub(crate) client: Arc<reqwest::Client>,
    pub(crate) proxy: Option<crate::config::HttpProxyConfig>,
    pub(crate) search_limit: usize,
    pub(crate) nuid: String,
    pub(crate) device_id: String,
}

impl NeteaseSource {
    pub fn new(
        config: Option<crate::config::NeteaseMusicConfig>,
        client: Arc<reqwest::Client>,
    ) -> Result<Self, String> {
        let cfg = config.ok_or("Netease Music configuration is missing")?;

        let mut rng = rand::thread_rng();
        let nuid: String = (0..16)
            .map(|_| format!("{:02x}", rng.r#gen::<u8>()))
            .collect::<Vec<String>>()
            .join("");

        let device_id: String = (0..8)
            .map(|_| format!("{:02X}", rng.r#gen::<u8>()))
            .collect::<Vec<String>>()
            .join("");

        Ok(Self {
            client,
            proxy: cfg.proxy,
            search_limit: cfg.search_limit,
            nuid,
            device_id,
        })
    }
}

#[async_trait]
impl SourcePlugin for NeteaseSource {
    fn name(&self) -> &str {
        "netease"
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
        vec!["nmsearch:", "ncsearch:"]
    }

    fn rec_prefixes(&self) -> Vec<&str> {
        vec!["nmrec:", "ncrec:"]
    }

    async fn load(
        &self,
        identifier: &str,
        _routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> LoadResult {
        for prefix in self.search_prefixes() {
            if let Some(query) = identifier.strip_prefix(prefix) {
                return manager::search_tracks(
                    &self.client,
                    &self.nuid,
                    &self.device_id,
                    query,
                    self.search_limit,
                )
                .await;
            }
        }

        for prefix in self.rec_prefixes() {
            if let Some(query) = identifier.strip_prefix(prefix) {
                return manager::fetch_recommendations(
                    &self.client,
                    &self.nuid,
                    &self.device_id,
                    query,
                )
                .await;
            }
        }

        if let Some(caps) = url_regex().captures(identifier) {
            let type_ = caps.name("type").map(|m| m.as_str()).unwrap_or("");
            let id = caps.name("id").map(|m| m.as_str()).unwrap_or("");

            match type_ {
                "song" => {
                    if let Some(detail) =
                        manager::fetch_track_detail(&self.client, &self.nuid, &self.device_id, id)
                            .await
                        && let Some(song) = detail.songs.first()
                        && let Some(track) = manager::parse_track(song)
                    {
                        return LoadResult::Track(track);
                    }
                }
                "album" => {
                    return manager::fetch_album(&self.client, &self.nuid, &self.device_id, id)
                        .await;
                }
                "playlist" => {
                    return manager::fetch_playlist(&self.client, &self.nuid, &self.device_id, id)
                        .await;
                }
                "artist" => {
                    return manager::fetch_artist(&self.client, &self.nuid, &self.device_id, id)
                        .await;
                }
                _ => {}
            }

            return LoadResult::Empty {};
        }

        if identifier.chars().all(|c| c.is_ascii_digit())
            && !identifier.is_empty()
            && let Some(detail) =
                manager::fetch_track_detail(&self.client, &self.nuid, &self.device_id, identifier)
                    .await
            && let Some(song) = detail.songs.first()
            && let Some(track) = manager::parse_track(song)
        {
            return LoadResult::Track(track);
        }

        manager::search_tracks(
            &self.client,
            &self.nuid,
            &self.device_id,
            identifier,
            self.search_limit,
        )
        .await
    }

    async fn load_search(
        &self,
        query: &str,
        _types: &[String],
        _routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<SearchResult> {
        let mut q = query;
        for prefix in self.search_prefixes() {
            if let Some(stripped) = query.strip_prefix(prefix) {
                q = stripped;
                break;
            }
        }

        manager::search_full(
            &self.client,
            &self.nuid,
            &self.device_id,
            q,
            _types,
            self.search_limit,
        )
        .await
    }

    async fn get_track(
        &self,
        identifier: &str,
        routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<BoxedTrack> {
        let id = url_regex()
            .captures(identifier)
            .and_then(|caps| caps.name("id"))
            .map(|m| m.as_str())
            .unwrap_or(identifier);

        debug!("Netease: Resolving track ID: {}", id);

        let mut stream_url = None;
        let mut fallback_early = false;

        let qualities = [
            ("aac", "standard"),
            ("aac", "higher"),
            ("aac", "exhigh"),
            ("aac", "lossless"),
            ("aac", "hires"),
            ("aac", "jymaster"),
            ("aac", "sky"),
            ("aac", "jyeffect"),
            ("aac", "jylive"),
            ("mp3", "standard"),
            ("mp3", "higher"),
            ("mp3", "exhigh"),
            ("mp3", "lossless"),
            ("mp3", "hires"),
            ("mp3", "jymaster"),
            ("mp3", "sky"),
            ("mp3", "jyeffect"),
            ("mp3", "jylive"),
        ];

        let mut first_code: Option<i64> = None;

        for (format, level) in qualities {
            match manager::fetch_track_url(
                &self.client,
                &self.nuid,
                &self.device_id,
                id,
                level,
                format,
            )
            .await
            {
                manager::TrackUrlResult::Success(url)
                    if manager::check_url(&self.client, &url).await =>
                {
                    stream_url = Some(url);
                    break;
                }
                manager::TrackUrlResult::Code(-110) => {
                    fallback_early = true;
                    break;
                }
                manager::TrackUrlResult::Trial => {
                    debug!("Netease: Track {} is trial-only, skipping quality loop", id);
                    fallback_early = true;
                    break;
                }
                manager::TrackUrlResult::Code(c) => {
                    first_code = first_code.or(Some(c));
                    continue;
                }
                _ => continue,
            }
        }

        if stream_url.is_none() || fallback_early {
            for br in ["320000", "128000"] {
                if let Some(url) =
                    manager::fetch_track_url_legacy(&self.client, &self.device_id, id, br).await
                    && !url.is_empty()
                    && manager::check_url(&self.client, &url).await
                {
                    stream_url = Some(url);
                    break;
                }
            }
        }

        if stream_url.is_none() {
            debug!(
                "Netease: Failed to resolve playback URL for track ID: {}",
                id
            );
        }

        stream_url.map(|url| {
            Box::new(track::NeteaseTrack {
                stream_url: url,
                proxy: self.proxy.clone(),
                local_addr: routeplanner.and_then(|rp| rp.get_address()),
            }) as BoxedTrack
        })
    }

    fn get_proxy_config(&self) -> Option<crate::config::HttpProxyConfig> {
        self.proxy.clone()
    }
}
