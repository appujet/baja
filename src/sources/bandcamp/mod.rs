use std::sync::Arc;

use async_trait::async_trait;
use regex::Regex;
use serde_json::{Value, json};
use tracing::error;

use crate::{
    protocol::tracks::{LoadResult, PlaylistData, PlaylistInfo, Track, TrackInfo},
    sources::{SourcePlugin, plugin::BoxedTrack},
};

pub mod track;

pub struct BandcampSource {
    client: reqwest::Client,
    pattern: Regex,
    identifier_pattern: Regex,
    search_prefixes: Vec<String>,
    search_limit: usize,
}

impl BandcampSource {
    pub fn new(config: Option<crate::configs::BandcampConfig>) -> Result<Self, String> {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::USER_AGENT,
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36".parse().unwrap()
        );

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .build()
            .map_err(|e| e.to_string())?;

        Ok(Self {
            client,
            pattern: Regex::new(r"(?i)^https?://(?P<subdomain>[^/]+)\.bandcamp\.com/(?P<type>track|album)/(?P<slug>[^/?]+)").unwrap(),
            identifier_pattern: Regex::new(r"^(?P<subdomain>[^:]+):(?P<slug>[^:]+)$").unwrap(),
            search_prefixes: vec!["bcsearch:".to_string()],
            search_limit: config.map(|c| c.search_limit).unwrap_or(10),
        })
    }

    async fn search(&self, query: &str) -> LoadResult {
        let url = format!(
            "https://bandcamp.com/search?q={}&item_type=t&from=results",
            urlencoding::encode(query)
        );

        let resp = match self.client.get(url).send().await {
            Ok(r) => r,
            Err(e) => {
                error!("Bandcamp search request failed: {}", e);
                return LoadResult::Empty {};
            }
        };

        if !resp.status().is_success() {
            return LoadResult::Empty {};
        }

        let body = match resp.text().await {
            Ok(t) => t,
            Err(_) => return LoadResult::Empty {},
        };

        let result_blocks_re =
            Regex::new(r"(?s)<li class=.searchresult data-search.[\s\S]*?</li>").unwrap();
        let url_re = Regex::new(r#"<a class="artcont" href="([^"]+)">"#).unwrap();
        let title_re =
            Regex::new(r#"(?s)<div class="heading">\s*<a[^>]*>\s*(.+?)\s*</a>"#).unwrap();
        let subhead_re = Regex::new(r#"(?s)<div class="subhead">([\s\S]*?)</div>"#).unwrap();
        let artwork_re = Regex::new(r#"(?s)<div class="art">\s*<img src="([^"]+)""#).unwrap();

        let mut tracks = Vec::new();
        for block in result_blocks_re.find_iter(&body) {
            let block_str = block.as_str();

            let url_match = url_re.captures(block_str);
            let title_match = title_re.captures(block_str);
            let subhead_match = subhead_re.captures(block_str);
            let artwork_match = artwork_re.captures(block_str);

            if let (Some(url_m), Some(title_m), Some(subhead_m)) =
                (url_match, title_match, subhead_match)
            {
                let uri = url_m[1].split('?').next().unwrap_or(&url_m[1]).to_string();
                let title = title_m[1].trim().to_string();
                let subhead = subhead_m[1].trim();
                let artist = subhead
                    .split(" de ")
                    .last()
                    .unwrap_or(subhead)
                    .trim()
                    .to_string();
                let artwork_url = artwork_match.map(|m| m[1].to_string());

                let info = TrackInfo {
                    identifier: self.get_identifier_from_url(&uri),
                    is_seekable: true,
                    author: artist,
                    length: 0,
                    is_stream: false,
                    position: 0,
                    title,
                    uri: Some(uri),
                    artwork_url,
                    isrc: None,
                    source_name: "bandcamp".to_string(),
                };
                tracks.push(Track::new(info));

                if tracks.len() >= self.search_limit {
                    break;
                }
            }
        }

        if tracks.is_empty() {
            return LoadResult::Empty {};
        }

        LoadResult::Search(tracks)
    }

    async fn resolve(&self, url: &str) -> LoadResult {
        let (tralbum_data, _) = match self.fetch_track_data(url).await {
            Some(d) => d,
            None => return LoadResult::Empty {},
        };

        let artist = tralbum_data["artist"]
            .as_str()
            .unwrap_or("Unknown Artist")
            .to_string();
        let art_id = tralbum_data["art_id"].as_u64();
        let artwork_url = art_id.map(|id| format!("https://f4.bcbits.com/img/a{}_10.jpg", id));

        if let Some(trackinfo) = tralbum_data["trackinfo"].as_array() {
            if trackinfo.len() > 1 {
                let mut tracks = Vec::new();
                for item in trackinfo {
                    let title = match item["title"].as_str() {
                        Some(t) => t.to_string(),
                        None => continue,
                    };
                    let track_url_suffix = item["title_link"].as_str();
                    if let Some(suffix) = track_url_suffix {
                        let track_url = if suffix.starts_with("http") {
                            suffix.to_string()
                        } else {
                            let base = url.split(".bandcamp.com").next().unwrap_or("");
                            format!("{}.bandcamp.com{}", base, suffix)
                        };

                        let duration = (item["duration"].as_f64().unwrap_or(0.0) * 1000.0) as u64;
                        let identifier = item["track_id"]
                            .as_u64()
                            .or(item["id"].as_u64())
                            .map(|id| id.to_string())
                            .unwrap_or_else(|| self.get_identifier_from_url(&track_url));

                        tracks.push(Track::new(TrackInfo {
                            identifier,
                            is_seekable: true,
                            author: artist.clone(),
                            length: duration,
                            is_stream: false,
                            position: 0,
                            title,
                            uri: Some(track_url),
                            artwork_url: artwork_url.clone(),
                            isrc: None,
                            source_name: "bandcamp".to_string(),
                        }));
                    }
                }

                let playlist_name = tralbum_data["current"]["title"]
                    .as_str()
                    .unwrap_or("Bandcamp Album")
                    .to_string();

                return LoadResult::Playlist(PlaylistData {
                    info: PlaylistInfo {
                        name: playlist_name,
                        selected_track: 0,
                    },
                    plugin_info: json!({}),
                    tracks,
                });
            } else if let Some(track_data) = trackinfo.first() {
                let title = track_data["title"]
                    .as_str()
                    .unwrap_or("Unknown Title")
                    .to_string();
                let duration = (track_data["duration"].as_f64().unwrap_or(0.0) * 1000.0) as u64;
                let identifier = track_data["track_id"]
                    .as_u64()
                    .or(track_data["id"].as_u64())
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| self.get_identifier_from_url(url));

                return LoadResult::Track(Track::new(TrackInfo {
                    identifier,
                    is_seekable: true,
                    author: artist,
                    length: duration,
                    is_stream: false,
                    position: 0,
                    title,
                    uri: Some(url.to_string()),
                    artwork_url,
                    isrc: None,
                    source_name: "bandcamp".to_string(),
                }));
            }
        }

        LoadResult::Empty {}
    }

    async fn fetch_track_data(&self, url: &str) -> Option<(Value, Option<String>)> {
        let resp = self.client.get(url).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }

        let body = resp.text().await.ok()?;

        let tralbum_re = Regex::new(r#"data-tralbum=["'](.+?)["']"#).unwrap();
        let tralbum_data = if let Some(match_cap) = tralbum_re.captures(&body) {
            let decoded = match_cap[1].replace("&quot;", "\"");
            serde_json::from_str(&decoded).ok()?
        } else {
            return None;
        };

        let stream_re = Regex::new(r"https?://t4\.bcbits\.com/stream/[a-zA-Z0-9]+/mp3-128/\d+\?p=\d+&amp;ts=\d+&amp;t=[a-zA-Z0-9]+&amp;token=\d+_[a-zA-Z0-9]+").unwrap();
        let stream_url = stream_re
            .find(&body)
            .map(|m| m.as_str().replace("&amp;", "&"));

        Some((tralbum_data, stream_url))
    }

    fn get_identifier_from_url(&self, url: &str) -> String {
        if let Some(caps) = self.pattern.captures(url) {
            return format!("{}:{}", &caps["subdomain"], &caps["slug"]);
        }
        url.to_string()
    }
}

#[async_trait]
impl SourcePlugin for BandcampSource {
    fn name(&self) -> &str {
        "bandcamp"
    }

    fn can_handle(&self, identifier: &str) -> bool {
        self.search_prefixes
            .iter()
            .any(|p| identifier.starts_with(p))
            || self.pattern.is_match(identifier)
            || self.identifier_pattern.is_match(identifier)
    }

    fn search_prefixes(&self) -> Vec<&str> {
        self.search_prefixes.iter().map(|s| s.as_str()).collect()
    }

    async fn load(
        &self,
        identifier: &str,
        _routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> LoadResult {
        for prefix in &self.search_prefixes {
            if identifier.starts_with(prefix) {
                return self.search(&identifier[prefix.len()..]).await;
            }
        }

        if self.pattern.is_match(identifier) {
            return self.resolve(identifier).await;
        }

        if let Some(caps) = self.identifier_pattern.captures(identifier) {
            let url = format!(
                "https://{}.bandcamp.com/track/{}",
                &caps["subdomain"], &caps["slug"]
            );
            return self.resolve(&url).await;
        }

        LoadResult::Empty {}
    }

    async fn get_track(
        &self,
        identifier: &str,
        routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<BoxedTrack> {
        let url = if identifier.starts_with("http") {
            identifier.to_string()
        } else if let Some(caps) = self.identifier_pattern.captures(identifier) {
            format!(
                "https://{}.bandcamp.com/track/{}",
                &caps["subdomain"], &caps["slug"]
            )
        } else {
            return None;
        };

        let (_, stream_url_opt) = self.fetch_track_data(&url).await?;
        let stream_url = stream_url_opt?;

        Some(Box::new(track::BandcampTrack {
            client: self.client.clone(),
            uri: url,
            stream_url: Some(stream_url),
            local_addr: routeplanner.and_then(|rp| rp.get_address()),
        }))
    }
}
