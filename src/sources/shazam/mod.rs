use std::sync::Arc;

use async_trait::async_trait;
use regex::Regex;
use reqwest::header::USER_AGENT;
use serde_json::Value;
use tracing::error;

use crate::{
    protocol::tracks::{LoadResult, Track, TrackInfo},
    sources::SourcePlugin,
};

const SEARCH_URL: &str = "https://www.shazam.com/services/amapi/v1/catalog/US/search";

pub struct ShazamSource {
    client: Arc<reqwest::Client>,
    search_prefixes: Vec<String>,
    url_regex: Regex,
    og_title_regex: Regex,
    search_limit: usize,
}

impl ShazamSource {
    pub fn new(
        config: &crate::configs::Config,
        client: Arc<reqwest::Client>,
    ) -> Result<Self, String> {
        Ok(Self {
            client,
            search_prefixes: vec!["shsearch:".to_string(), "szsearch:".to_string()],
            url_regex: Regex::new(r"https?://(?:www\.)?shazam\.com/song/\d+(?:/[^/?#]+)?")
                .expect("shazam URL regex is a valid literal"),
            og_title_regex: Regex::new(r"^(.+?) - (.+?):")
                .expect("shazam og:title regex is a valid literal"),
            search_limit: config.shazam.as_ref().map(|c| c.search_limit).unwrap_or(10),
        })
    }

    async fn search(&self, query: &str) -> LoadResult {
        let url = format!(
            "{}?types=songs&term={}&limit={}",
            SEARCH_URL,
            urlencoding::encode(query),
            self.search_limit
        );

        let resp = match self.base_request(self.client.get(&url)).send().await {
            Ok(r) => r,
            Err(e) => {
                error!("Shazam search request failed: {}", e);
                return LoadResult::Empty {};
            }
        };

        if !resp.status().is_success() {
            return LoadResult::Empty {};
        }

        let data: Value = match resp.json().await {
            Ok(v) => v,
            Err(e) => {
                error!("Failed to parse Shazam search JSON: {}", e);
                return LoadResult::Empty {};
            }
        };

        let songs = data
            .pointer("/results/songs/data")
            .and_then(|v| v.as_array());
        let Some(songs) = songs else {
            return LoadResult::Empty {};
        };

        let mut tracks = Vec::new();
        for item in songs {
            if let Some(track) = self.build_track(item) {
                tracks.push(track);
            }
        }

        if tracks.is_empty() {
            LoadResult::Empty {}
        } else {
            LoadResult::Search(tracks)
        }
    }

    fn build_track(&self, item: &Value) -> Option<Track> {
        let attributes = item.get("attributes")?;
        let id = item.get("id")?.as_str()?.to_string();

        let title = attributes
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown")
            .to_string();
        let author = attributes
            .get("artistName")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown")
            .to_string();
        let length = attributes
            .get("durationInMillis")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let isrc = attributes
            .get("isrc")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let artwork_url = attributes
            .get("artwork")
            .and_then(|a| a.get("url"))
            .and_then(|v| v.as_str())
            .map(|s| s.replace("{w}", "1000").replace("{h}", "1000"));

        let uri = attributes
            .get("url")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let mut track = Track::new(TrackInfo {
            identifier: id,
            is_seekable: true,
            author,
            length,
            is_stream: false,
            position: 0,
            title,
            uri: uri.clone(),
            artwork_url,
            isrc,
            source_name: "shazam".to_string(),
        });

        track.plugin_info = serde_json::json!({
            "albumName": null,
            "albumUrl": null,
            "artistUrl": null,
            "artistArtworkUrl": null,
            "previewUrl": null,
            "isPreview": false
        });

        Some(track)
    }

    async fn resolve_url(&self, url: &str) -> LoadResult {
        let resp = match self.base_request(self.client.get(url)).send().await {
            Ok(r) => r,
            Err(e) => {
                error!("Shazam resolve request failed: {}", e);
                return LoadResult::Empty {};
            }
        };

        if !resp.status().is_success() {
            return LoadResult::Empty {};
        }

        let html = match resp.text().await {
            Ok(t) => t,
            Err(e) => {
                error!("Failed to read Shazam HTML: {}", e);
                return LoadResult::Empty {};
            }
        };

        let title = self.extract_text_after_class(&html, "NewTrackPageHeader_trackTitle__");
        let artist = self.extract_text_after_class(&html, "TrackPageArtistLink_artistNameText__");
        let artwork_url = self.extract_artwork(&html);
        let isrc = self.extract_isrc(&html);
        let duration_ms = self.extract_duration(&html);
        let apple_music_url =
            self.extract_href_starting_with(&html, "https://www.shazam.com/applemusic/song/");

        let mut final_title = title.unwrap_or_else(|| "Unknown".to_string());
        let mut final_artist = artist.unwrap_or_else(|| "Unknown".to_string());

        if final_title == "Unknown" {
            if let Some(og_title) = self.extract_meta_content(&html, "og:title") {
                // Use the pre-compiled field regex — never allocate inside a request path.
                if let Some(caps) = self.og_title_regex.captures(&og_title) {
                    // Both groups are guaranteed by the regex structure, but use
                    // safe accessors to avoid panics if the input is malformed.
                    final_title = caps
                        .get(1)
                        .map(|m| m.as_str().to_string())
                        .unwrap_or_else(|| og_title.clone());
                    final_artist = caps
                        .get(2)
                        .map(|m| m.as_str().to_string())
                        .unwrap_or_else(|| "Unknown".to_string());
                } else {
                    final_title = og_title;
                }
            }
        }

        if final_title == "Unknown" && apple_music_url.is_none() {
            return LoadResult::Empty {};
        }

        let clean_url = if url.ends_with('/') {
            &url[..url.len() - 1]
        } else {
            url
        };
        let identifier = clean_url.split('/').last().unwrap_or("unknown").to_string();

        let mut track = Track::new(TrackInfo {
            identifier,
            is_seekable: true,
            author: final_artist,
            length: duration_ms,
            is_stream: false,
            position: 0,
            title: final_title,
            uri: Some(url.to_string()),
            artwork_url: artwork_url.or_else(|| self.extract_meta_content(&html, "og:image")),
            isrc,
            source_name: "shazam".to_string(),
        });

        track.plugin_info = serde_json::json!({
            "albumName": null,
            "albumUrl": null,
            "artistUrl": null,
            "artistArtworkUrl": null,
            "previewUrl": null,
            "isPreview": false
        });

        LoadResult::Track(track)
    }

    fn extract_text_after_class(&self, html: &str, class_part: &str) -> Option<String> {
        let mut from = 0;
        while let Some(c) = html[from..].find("class=\"") {
            let c = from + c;
            let q = html[c + 7..].find('"').map(|i| c + 7 + i)?;
            let cls = &html[c + 7..q];
            if cls.contains(class_part) {
                let gt = html[q..].find('>').map(|i| q + i)?;
                let lt = html[gt + 1..].find('<').map(|i| gt + 1 + i)?;
                let text = html[gt + 1..lt].trim().to_string();
                if !text.is_empty() {
                    return Some(text);
                }
            }
            from = q + 1;
        }
        None
    }

    fn extract_href_starting_with(&self, html: &str, prefix: &str) -> Option<String> {
        let pattern = format!("href=\"{}\"", prefix);
        if let Some(i) = html.find(&pattern) {
            let start = i + 6;
            if let Some(end) = html[start..].find('"') {
                return Some(html[start..start + end].to_string());
            }
        }
        None
    }

    fn extract_artwork(&self, html: &str) -> Option<String> {
        if let Some(og) = self.extract_meta_content(html, "og:image") {
            return Some(og);
        }

        let mut alt_idx = html.find("alt=\"album cover\"");
        if alt_idx.is_none() {
            alt_idx = html.find("alt=\"song thumbnail\"");
        }
        let alt_idx = alt_idx?;

        let img_start = html[..alt_idx].rfind("<img")?;
        let img_end = html[alt_idx..].find('>')? + alt_idx;
        let tag = &html[img_start..img_end + 1];

        if let Some(s) = tag.find("srcset=\"") {
            let val_start = s + 8;
            if let Some(val_end) = tag[val_start..].find('"') {
                let srcset = &tag[val_start..val_start + val_end];
                let space = srcset.find(' ').unwrap_or(srcset.len());
                return Some(srcset[..space].to_string());
            }
        }
        None
    }

    fn extract_isrc(&self, html: &str) -> Option<String> {
        let tokens = ["\"isrc\"", "\\\"isrc\\\""];
        for token in tokens {
            let mut from = 0;
            while let Some(at) = html[from..].find(token) {
                let at = from + at;
                from = at + token.len();
                let mut i = html[from..].find(':')? + from + 1;

                while i < html.len() {
                    let bytes = html.as_bytes();
                    if bytes[i] != b' '
                        && bytes[i] != b'\t'
                        && bytes[i] != b'\n'
                        && bytes[i] != b'\r'
                    {
                        break;
                    }
                    i += 1;
                }

                while i < html.len() && html.as_bytes()[i] == b'\\' {
                    i += 1;
                }
                if i >= html.len() || html.as_bytes()[i] != b'"' {
                    continue;
                }
                i += 1;

                if i + 12 > html.len() {
                    continue;
                }

                let isrc_cand = &html[i..i + 12];
                if self.is_valid_isrc(isrc_cand) {
                    return Some(isrc_cand.to_string());
                }
            }
        }
        None
    }

    fn is_valid_isrc(&self, s: &str) -> bool {
        if s.len() != 12 {
            return false;
        }
        let b = s.as_bytes();
        // 2 upper
        if !b[0].is_ascii_uppercase() || !b[1].is_ascii_uppercase() {
            return false;
        }
        // 3 upper or digit
        for &c in &b[2..5] {
            if !c.is_ascii_uppercase() && !c.is_ascii_digit() {
                return false;
            }
        }
        // 7 digit
        for &c in &b[5..12] {
            if !c.is_ascii_digit() {
                return false;
            }
        }
        true
    }

    fn extract_duration(&self, html: &str) -> u64 {
        let needles = [
            "\"duration\":\"PT",
            "\"duration\": \"PT",
            "\\\"duration\\\":\\\"PT",
        ];
        for n in needles {
            if let Some(at) = html.find(n) {
                let start = at + n.len() - 2;
                let bracket = n.starts_with('\\');
                let end_char = if bracket { '\\' } else { '"' };
                if let Some(end) = html[start..].find(end_char) {
                    return self.parse_iso_duration(&html[start..start + end]);
                }
            }
        }
        0
    }

    fn parse_iso_duration(&self, iso: &str) -> u64 {
        if !iso.contains('T') {
            return 0;
        }
        let t_idx = iso.find('T').unwrap() + 1;
        let mut ms = 0.0;
        let mut current_num = String::new();

        for c in iso[t_idx..].chars() {
            if c.is_ascii_digit() || c == '.' {
                current_num.push(c);
            } else {
                let val = current_num.parse::<f64>().unwrap_or(0.0);
                match c {
                    'H' => ms += val * 3600000.0,
                    'M' => ms += val * 60000.0,
                    'S' => ms += val * 1000.0,
                    _ => break,
                }
                current_num.clear();
            }
        }
        ms as u64
    }

    fn extract_meta_content(&self, html: &str, prop: &str) -> Option<String> {
        let pattern = format!("<meta property=\"{}\" content=\"", prop);
        if let Some(i) = html.find(&pattern) {
            let start = i + pattern.len();
            if let Some(end) = html[start..].find('"') {
                return Some(html[start..start + end].to_string());
            }
        }
        None
    }

    pub fn base_request(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        builder.header(USER_AGENT, "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.0.0 Safari/537.36")
    }
}

#[async_trait]
impl SourcePlugin for ShazamSource {
    fn name(&self) -> &str {
        "shazam"
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

        if self.url_regex.is_match(identifier) {
            return self.resolve_url(identifier).await;
        }

        LoadResult::Empty {}
    }

    async fn get_track(
        &self,
        _identifier: &str,
        _routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<crate::sources::plugin::BoxedTrack> {
        None
    }
}
