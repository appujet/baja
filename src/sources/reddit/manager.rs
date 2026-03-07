use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use regex::Regex;
use serde_json::Value;

use super::track::RedditTrack;
use crate::{
    protocol::tracks::{LoadResult, Track, TrackInfo},
    sources::plugin::{BoxedTrack, SourcePlugin},
};

static PATH_EXTRACTOR: OnceLock<Regex> = OnceLock::new();
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36";

pub struct RedditSource {
    http: Arc<reqwest::Client>,
}

impl RedditSource {
    pub fn new(
        _config: Option<crate::config::sources::RedditConfig>,
        http: Arc<reqwest::Client>,
    ) -> Result<Self, String> {
        Ok(Self { http })
    }

    async fn acquire_metadata_packet(
        &self,
        resource_url: &str,
    ) -> Option<(TrackInfo, Option<String>)> {
        let mut target_endpoint = resource_url.to_owned();

        if resource_url.contains("/s/") || resource_url.contains("/video/") {
            target_endpoint = self.resolve_canonical_link(resource_url).await?;
        }

        let metadata_url = if target_endpoint.ends_with('/') {
            format!("{}.json", &target_endpoint[..target_endpoint.len() - 1])
        } else {
            format!("{}.json", target_endpoint)
        };

        let raw_json = self.fetch_payload(&metadata_url).await?;

        let post_listing = match raw_json.as_array().and_then(|a| a.first()) {
            Some(l) => l,
            None => return None,
        };

        let post_data = match post_listing["data"]["children"]
            .as_array()
            .and_then(|a| a.first())
            .and_then(|c| c["data"].as_object())
        {
            Some(d) => d,
            None => return None,
        };

        let entry_name = match post_data.get("title").and_then(|v| v.as_str()) {
            Some(t) => self.unescape_html(t),
            None => return None,
        };
        let creator = format!(
            "u/{}",
            post_data
                .get("author")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
        );
        let preview_img = post_data
            .get("preview")
            .and_then(|p| p.get("images"))
            .and_then(|i| i.as_array())
            .and_then(|i| i.first())
            .and_then(|i| i.get("source"))
            .and_then(|s| s.get("url"))
            .and_then(|u| u.as_str())
            .or_else(|| post_data.get("thumbnail").and_then(|v| v.as_str()))
            .filter(|s| s.starts_with("http"))
            .map(|s| self.unescape_html(s));

        let media_spec = match post_data
            .get("secure_media")
            .and_then(|sm| sm.get("reddit_video"))
            .or_else(|| post_data.get("media").and_then(|m| m.get("reddit_video")))
        {
            Some(ms) => ms,
            None => return None,
        };

        let duration = match media_spec.get("duration").and_then(|v| v.as_f64()) {
            Some(d) => (d * 1000.0) as u64,
            None => 0,
        };

        let base_stream = match media_spec.get("fallback_url").and_then(|v| v.as_str()) {
            Some(u) => u.split('?').next().unwrap_or(u),
            None => return None,
        };

        let rid = self.identify_resource(resource_url);
        let audio_stream = self.discover_audio_asset(base_stream).await;

        Some((
            TrackInfo {
                identifier: rid,
                is_seekable: true,
                author: creator,
                length: duration,
                is_stream: false,
                position: 0,
                title: entry_name,
                uri: Some(resource_url.to_owned()),
                artwork_url: preview_img,
                isrc: None,
                source_name: "reddit".to_owned(),
            },
            audio_stream,
        ))
    }

    async fn resolve_canonical_link(&self, url: &str) -> Option<String> {
        let outcome = self
            .http
            .head(url)
            .header(reqwest::header::USER_AGENT, USER_AGENT)
            .send()
            .await
            .ok()?;

        if outcome.status().is_redirection() {
            outcome
                .headers()
                .get(reqwest::header::LOCATION)?
                .to_str()
                .ok()
                .map(|l| l.split('?').next().unwrap_or(l).to_owned())
        } else {
            Some(url.to_owned())
        }
    }

    async fn fetch_payload(&self, url: &str) -> Option<Value> {
        self.http
            .get(url)
            .header(reqwest::header::USER_AGENT, USER_AGENT)
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await
            .ok()?
            .json()
            .await
            .ok()
    }

    async fn discover_audio_asset(&self, primary_url: &str) -> Option<String> {
        let anchor = primary_url.split('_').next()?;

        let pool = vec![
            format!("{}_audio.mp4", anchor),
            format!("{}_AUDIO_128.mp4", anchor),
            format!("{}_audio.mp3", anchor),
            primary_url.replace("DASH_", "DASH_audio"),
        ];

        for probe_url in pool {
            if self
                .http
                .head(&probe_url)
                .header(reqwest::header::USER_AGENT, USER_AGENT)
                .send()
                .await
                .is_ok_and(|res| res.status().is_success())
            {
                return Some(probe_url);
            }
        }
        None
    }

    fn identify_resource(&self, link: &str) -> String {
        let pattern =
            PATH_EXTRACTOR.get_or_init(|| Regex::new(r"/(?:comments|video|s)/([^/?#]+)").unwrap());

        if let Some(hits) = pattern.captures(link) {
            return hits[1].to_owned();
        }

        link.to_owned()
    }

    fn unescape_html(&self, input: &str) -> String {
        let mut result = input.to_owned();
        loop {
            let next = result
                .replace("&amp;", "&")
                .replace("&lt;", "<")
                .replace("&gt;", ">")
                .replace("&quot;", "\"")
                .replace("&#39;", "'")
                .replace("&apos;", "'");

            if next == result {
                break;
            }
            result = next;
        }
        result
    }
}

#[async_trait]
impl SourcePlugin for RedditSource {
    fn name(&self) -> &str {
        "reddit"
    }

    fn can_handle(&self, identifier: &str) -> bool {
        identifier.contains("reddit.com/") || identifier.contains("v.redd.it/")
    }

    async fn load(
        &self,
        identifier: &str,
        _routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> LoadResult {
        match self.acquire_metadata_packet(identifier).await {
            Some((meta, _)) => LoadResult::Track(Track::new(meta)),
            None => LoadResult::Empty {},
        }
    }

    async fn get_track(
        &self,
        identifier: &str,
        routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<BoxedTrack> {
        let (meta, audio_stream) = self.acquire_metadata_packet(identifier).await?;

        Some(Box::new(RedditTrack {
            client: self.http.clone(),
            uri: meta.uri.unwrap_or_else(|| identifier.to_owned()),
            audio_url: audio_stream,
            local_addr: routeplanner.and_then(|rp| rp.get_address()),
        }))
    }
}
