

use super::{LastFMSource, helpers::{get_json, unescape_html}};
use crate::protocol::tracks::{LoadResult, Track, TrackInfo};

impl LastFMSource {
    pub async fn search_tracks(&self, query: &str) -> LoadResult {
        if let Some(ref key) = self.api_key {
            self.search_api(query, key).await
        } else {
            self.search_scraping(query).await
        }
    }

    async fn search_api(&self, query: &str, api_key: &str) -> LoadResult {
        let url = format!(
            "https://ws.audioscrobbler.com/2.0/?method=track.search&track={}&api_key={}&limit={}&format=json",
            urlencoding::encode(query),
            api_key,
            self.search_limit
        );

        let json = match get_json(&self.http, &url).await {
            Some(j) => j,
            None => return LoadResult::Empty {},
        };

        let tracks = match json["results"]["trackmatches"]["track"].as_array() {
            Some(t) => t,
            None => {
                tracing::debug!("Last.fm: API response missing trackmatches for search '{}'", query);
                return LoadResult::Empty {};
            }
        };

        let results: Vec<Track> = tracks
            .iter()
            .map(|t| {
                let title = t["name"].as_str().unwrap_or("Unknown").to_owned();
                let artist = t["artist"].as_str().unwrap_or("Unknown").to_owned();
                let uri = crate::sources::lastfm::construct_track_url(&artist, &title);

                let artwork_url = t["image"]
                    .as_array()
                    .and_then(|images| images.last())
                    .and_then(|img| img["#text"].as_str())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.replace("/34s/", "/300x300/"));

                Track::new(TrackInfo {
                    identifier: uri.clone(),
                    is_seekable: true,
                    author: artist,
                    title,
                    uri: Some(uri),
                    artwork_url,
                    source_name: "lastfm".to_owned(),
                    ..Default::default()
                })
            })
            .collect();

        if results.is_empty() {
            tracing::debug!("Last.fm: API search returned no tracks for '{}'", query);
            LoadResult::Empty {}
        } else {
            LoadResult::Search(results)
        }
    }

    async fn search_scraping(&self, query: &str) -> LoadResult {
        let url = format!(
            "https://www.last.fm/search/tracks?q={}",
            urlencoding::encode(query)
        );

        let body = match self.http.get(&url).send().await {
            Ok(r) => r.text().await.unwrap_or_else(|e| {
                tracing::debug!("Last.fm: failed to get response text for search scraping '{}': {}", query, e);
                Default::default()
            }),
            Err(e) => {
                tracing::debug!("Last.fm: search scraping request failed for '{}': {}", query, e);
                return LoadResult::Empty {};
            }
        };

        let mut results = Vec::new();
        for caps in crate::sources::lastfm::search_regex().captures_iter(&body) {
            let artwork_url = caps.get(1).map(|m| m.as_str().replace("/64s/", "/300x300/"));
            let title = unescape_html(caps.get(2).map(|m| m.as_str()).unwrap_or("Unknown"));
            let artist = unescape_html(caps.get(4).map(|m| m.as_str()).unwrap_or("Unknown"));
 
            let full_url = crate::sources::lastfm::construct_track_url(&artist, &title);
 
            results.push(Track::new(TrackInfo {
                identifier: full_url.clone(),
                is_seekable: true,
                author: artist,
                title,
                uri: Some(full_url),
                artwork_url,
                source_name: "lastfm".to_owned(),
                ..Default::default()
            }));

            if results.len() >= self.search_limit {
                break;
            }
        }

        if results.is_empty() {
            tracing::debug!("Last.fm: search scraping found no tracks for '{}' on page {}", query, url);
            LoadResult::Empty {}
        } else {
            LoadResult::Search(results)
        }
    }
}
