pub mod helpers;
pub mod metadata;
pub mod parser;
pub mod reader;
pub mod recommendations;
pub mod search;
pub mod token;
pub mod track;

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use regex::Regex;
use token::DeezerTokenTracker;
use track::DeezerTrack;

use crate::{
    protocol::tracks::LoadResult,
    sources::{SourcePlugin, plugin::PlayableTrack},
};

const PUBLIC_API_BASE: &str = "https://api.deezer.com";
const PRIVATE_API_BASE: &str = "https://www.deezer.com/ajax/gw-light.php";

pub(crate) const REC_ARTIST_PREFIX: &str = "artist=";
pub(crate) const REC_TRACK_PREFIX: &str = "track=";

fn url_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"https?://(?:www\.)?deezer\.com/(?:[a-z]+(?:-[a-z]+)?/)?(?<type>track|album|playlist|artist)/(?<id>\d+)").unwrap()
    })
}

fn share_url_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"https?://(?:deezer\.page\.link|link\.deezer\.com)/\S*").unwrap()
    })
}

pub struct DeezerSource {
    client: Arc<reqwest::Client>,
    config: crate::config::DeezerConfig,
    pub token_tracker: Arc<DeezerTokenTracker>,
}

const DECRYPTION_KEY_HASH: [u8; 32] = [
    52, 76, 41, 138, 120, 133, 48, 72, 198, 74, 16, 75, 82, 101, 186, 223, 15, 190, 111, 218, 176,
    71, 103, 11, 181, 136, 155, 247, 66, 203, 218, 240,
];

impl DeezerSource {
    pub fn new(
        config: crate::config::DeezerConfig,
        client: Arc<reqwest::Client>,
    ) -> Result<Self, String> {
        let mut arls = config.arls.clone().unwrap_or_default();
        arls.retain(|s| !s.is_empty());
        arls.sort();
        arls.dedup();

        if arls.is_empty() {
            return Err("Deezer arls must be set".to_owned());
        }

        if let Some(ref key) = config.master_decryption_key {
            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            hasher.update(key.as_bytes());
            if hasher.finalize().as_slice() != DECRYPTION_KEY_HASH {
                tracing::warn!("Deezer master decryption key is possibly invalid, playback may not work!");
            }
        }

        let token_tracker = Arc::new(DeezerTokenTracker::new(client.clone(), arls));

        Ok(Self {
            client,
            config,
            token_tracker,
        })
    }

    async fn resolve_share_url(&self, identifier: &str) -> Option<String> {
        let client = reqwest::Client::builder()
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.6094.0 Safari/537.36")
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .ok()?;

        let res = client.get(identifier).send().await.ok()?;
        if !res.status().is_redirection() {
            return None;
        }

        let loc = res.headers().get("location")?.to_str().ok()?;
        let mut url = loc.to_owned();

        if let Some(pos) = url.find("dest=") {
            let dest = &url[pos + 5..];
            let end = dest.find('&').unwrap_or(dest.len());
            if let Ok(decoded) = urlencoding::decode(&dest[..end]) {
                url = decoded.into_owned();
            }
        }

        if let Some(pos) = url.find('?') {
            url.truncate(pos);
        }

        if url.ends_with("/404") {
            return None;
        }

        Some(url)
    }
}

#[async_trait]
impl SourcePlugin for DeezerSource {
    fn name(&self) -> &str {
        "deezer"
    }

    fn can_handle(&self, identifier: &str) -> bool {
        self.search_prefixes()
            .iter()
            .any(|p| identifier.starts_with(p))
            || self
                .isrc_prefixes()
                .iter()
                .any(|p| identifier.starts_with(p))
            || self
                .rec_prefixes()
                .iter()
                .any(|p| identifier.starts_with(p))
            || share_url_regex().is_match(identifier)
            || url_regex().is_match(identifier)
    }

    fn search_prefixes(&self) -> Vec<&str> {
        vec!["dzsearch:"]
    }

    fn isrc_prefixes(&self) -> Vec<&str> {
        vec!["dzisrc:"]
    }

    fn rec_prefixes(&self) -> Vec<&str> {
        vec!["dzrec:"]
    }

    async fn load(
        &self,
        identifier: &str,
        routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> LoadResult {
        for prefix in self.search_prefixes() {
            if let Some(query) = identifier.strip_prefix(prefix) {
                return self.search(query).await;
            }
        }

        for prefix in self.isrc_prefixes() {
            if let Some(isrc) = identifier.strip_prefix(prefix) {
                if let Some(track) = self.get_track_by_isrc(isrc).await {
                    return LoadResult::Track(track);
                }
                return LoadResult::Empty {};
            }
        }

        for prefix in self.rec_prefixes() {
            if let Some(query) = identifier.strip_prefix(prefix) {
                return self.get_recommendations(query).await;
            }
        }

        if share_url_regex().is_match(identifier) {
            if let Some(resolved) = self.resolve_share_url(identifier).await {
                return self.load(&resolved, routeplanner).await;
            }
            return LoadResult::Empty {};
        }

        if let Some(caps) = url_regex().captures(identifier) {
            let type_ = caps.name("type").map(|m| m.as_str()).unwrap_or("");
            let id = caps.name("id").map(|m| m.as_str()).unwrap_or("");
            return match type_ {
                "track" => {
                    if let Some(json) = self.get_json_public(&format!("track/{id}")).await
                        && let Some(track) = self.parse_track(&json)
                    {
                        return LoadResult::Track(track);
                    }
                    LoadResult::Empty {}
                }
                "album" => self.get_album(id).await,
                "playlist" => self.get_playlist(id).await,
                "artist" => self.get_artist(id).await,
                _ => LoadResult::Empty {},
            };
        }

        LoadResult::Empty {}
    }

    async fn get_track(
        &self,
        identifier: &str,
        routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<Box<dyn PlayableTrack>> {
        let track_id = if let Some(caps) = url_regex().captures(identifier) {
            caps.name("id").map(|m| m.as_str())?.to_owned()
        } else {
            identifier.to_owned()
        };

        let resolved =
            track::verify_track_resolvable(&self.client, &track_id, &self.token_tracker).await;

        if resolved.is_none() {
            tracing::warn!("Deezer: no stream URL for track {track_id}, falling back to mirrors");
            return None;
        }

        Some(Box::new(DeezerTrack {
            client: self.client.clone(),
            track_id,
            token_tracker: self.token_tracker.clone(),
            master_key: self
                .config
                .master_decryption_key
                .clone()
                .unwrap_or_default(),
            local_addr: routeplanner.and_then(|rp| rp.get_address()),
            proxy: self.config.proxy.clone(),
        }))
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
