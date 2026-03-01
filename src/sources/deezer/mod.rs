pub mod helpers;
pub mod metadata;
pub mod parser;
pub mod reader;
pub mod recommendations;
pub mod search;
pub mod token;
pub mod track;

use std::sync::Arc;

use async_trait::async_trait;
use regex::Regex;
use token::DeezerTokenTracker;
use track::DeezerTrack;

use crate::{
    protocol::tracks::LoadResult,
    sources::{PlayableTrack, SourcePlugin, StreamInfo},
};

const PUBLIC_API_BASE: &str = "https://api.deezer.com";
const PRIVATE_API_BASE: &str = "https://www.deezer.com/ajax/gw-light.php";

pub struct DeezerSource {
    client: Arc<reqwest::Client>,
    config: crate::configs::DeezerConfig,
    pub token_tracker: Arc<DeezerTokenTracker>,
    url_regex: Regex,
    search_prefixes: Vec<String>,
    isrc_prefixes: Vec<String>,
    rec_prefixes: Vec<String>,
    rec_artist_prefix: String,
    rec_track_prefix: String,
    share_url_prefix: String,
}

impl DeezerSource {
    pub fn new(
        config: crate::configs::DeezerConfig,
        client: Arc<reqwest::Client>,
    ) -> Result<Self, String> {
        let mut arls = config.arls.clone().unwrap_or_default();
        arls.retain(|s| !s.is_empty());
        arls.sort();
        arls.dedup();

        if arls.is_empty() {
            return Err("Deezer arls must be set".to_string());
        }
        let token_tracker = Arc::new(DeezerTokenTracker::new(client.clone(), arls));

        Ok(Self {
      client,
      config,
      token_tracker,
      url_regex: Regex::new(r"https?://(?:www\.)?deezer\.com/(?:[a-z]+(?:-[a-z]+)?/)?(?<type>track|album|playlist|artist)/(?<id>\d+)").unwrap(),
      search_prefixes: vec!["dzsearch:".to_string()],
      isrc_prefixes: vec!["dzisrc:".to_string()],
      rec_prefixes: vec!["dzrec:".to_string()],
      rec_artist_prefix: "artist=".to_string(),
      rec_track_prefix: "track=".to_string(),
      share_url_prefix: "https://deezer.page.link/".to_string(),
    })
    }
}

#[async_trait]
impl SourcePlugin for DeezerSource {
    fn name(&self) -> &str {
        "deezer"
    }
    fn can_handle(&self, identifier: &str) -> bool {
        self.search_prefixes
            .iter()
            .any(|p| identifier.starts_with(p))
            || self.isrc_prefixes.iter().any(|p| identifier.starts_with(p))
            || self.rec_prefixes.iter().any(|p| identifier.starts_with(p))
            || identifier.starts_with(&self.share_url_prefix)
            || self.url_regex.is_match(identifier)
    }

    fn search_prefixes(&self) -> Vec<&str> {
        self.search_prefixes.iter().map(|s| s.as_str()).collect()
    }

    fn isrc_prefixes(&self) -> Vec<&str> {
        self.isrc_prefixes.iter().map(|s| s.as_str()).collect()
    }

    async fn load(
        &self,
        identifier: &str,
        routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> LoadResult {
        if let Some(prefix) = self
            .search_prefixes
            .iter()
            .find(|p| identifier.starts_with(*p))
        {
            return self.search(identifier.strip_prefix(prefix).unwrap()).await;
        }
        if let Some(prefix) = self
            .isrc_prefixes
            .iter()
            .find(|p| identifier.starts_with(*p))
        {
            if let Some(track) = self
                .get_track_by_isrc(identifier.strip_prefix(prefix).unwrap())
                .await
            {
                return LoadResult::Track(track);
            }
            return LoadResult::Empty {};
        }
        if let Some(prefix) = self
            .rec_prefixes
            .iter()
            .find(|p| identifier.starts_with(*p))
        {
            return self
                .get_recommendations(identifier.strip_prefix(prefix).unwrap())
                .await;
        }
        if identifier.starts_with(&self.share_url_prefix) {
            let client = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap_or_else(|_| (*self.client).clone());
            if let Ok(res) = client.get(identifier).send().await {
                if res.status().is_redirection() {
                    if let Some(loc) = res.headers().get("location").and_then(|l| l.to_str().ok()) {
                        if loc.starts_with("https://www.deezer.com/") {
                            return self.load(loc, routeplanner).await;
                        }
                    }
                }
            }
            return LoadResult::Empty {};
        }
        if let Some(caps) = self.url_regex.captures(identifier) {
            let type_ = caps.name("type").map(|m| m.as_str()).unwrap_or("");
            let id = caps.name("id").map(|m| m.as_str()).unwrap_or("");
            return match type_ {
                "track" => {
                    if let Some(json) = self.get_json_public(&format!("track/{}", id)).await {
                        if let Some(track) = self.parse_track(&json) {
                            return LoadResult::Track(track);
                        }
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
        let track_id = if let Some(caps) = self.url_regex.captures(identifier) {
            caps.name("id").map(|m| m.as_str())?.to_string()
        } else {
            identifier.to_string()
        };

        Some(Box::new(DeezerTrack {
            client: self.client.clone(),
            track_id,
            arl_index: 0, // get_token will rotate
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
        let q = if let Some(prefix) = self.search_prefixes.iter().find(|p| query.starts_with(*p)) {
            query.strip_prefix(prefix).unwrap()
        } else {
            query
        };

        self.get_autocomplete(q, types).await
    }

    async fn get_stream_url(&self, identifier: &str, _itag: Option<i64>) -> Option<StreamInfo> {
        let track_id = if let Some(caps) = self.url_regex.captures(identifier) {
            caps.name("id").map(|m| m.as_str())?.to_string()
        } else {
            identifier.to_string()
        };

        let tokens = self.token_tracker.get_token().await?;

        let api_url = format!(
            "https://www.deezer.com/ajax/gw-light.php?method=song.getData&input=3&api_version=1.0&api_token={}",
            tokens.api_token
        );
        let body = serde_json::json!({ "sng_id": track_id });
        let res = self
            .client
            .post(&api_url)
            .header(
                "Cookie",
                format!("sid={}; dzr_uniq_id={}", tokens.session_id, tokens.dzr_uniq_id),
            )
            .json(&body)
            .send()
            .await
            .ok()?;
        let json: serde_json::Value = res.json().await.ok()?;

        if json
            .get("error")
            .and_then(|v| v.as_array())
            .map(|a| !a.is_empty())
            .unwrap_or(false)
        {
            return None;
        }

        let track_token = json
            .get("results")
            .and_then(|r| r.get("TRACK_TOKEN"))
            .and_then(|v| v.as_str())?;

        let media_body = serde_json::json!({
            "license_token": tokens.license_token,
            "media": [{
                "type": "FULL",
                "formats": [
                    { "cipher": "BF_CBC_STRIPE", "format": "MP3_128" },
                    { "cipher": "BF_CBC_STRIPE", "format": "MP3_64" }
                ]
            }],
            "track_tokens": [track_token]
        });

        let res = self
            .client
            .post("https://media.deezer.com/v1/get_url")
            .json(&media_body)
            .send()
            .await
            .ok()?;
        let json: serde_json::Value = res.json().await.ok()?;

        let url = json
            .get("data")
            .and_then(|d| d.get(0))
            .and_then(|d| d.get("media"))
            .and_then(|m| m.get(0))
            .and_then(|m| m.get("sources"))
            .and_then(|s| s.get(0))
            .and_then(|s| s.get("url"))
            .and_then(|u| u.as_str())?;

        // NOTE: BF_CBC_STRIPE encrypted; decrypt each 2048-byte chunk with the per-track Blowfish key before playback.
        Some(StreamInfo {
            url: format!("deezer_encrypted:{}:{}", track_id, url),
            mime_type: "audio/mpeg".to_string(),
            protocol: "http".to_string(),
        })
    }
}
