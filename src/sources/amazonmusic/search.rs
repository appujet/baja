use std::sync::Arc;
use serde_json::Value;
use tracing::error;
use crate::protocol::tracks::{SearchResult, TrackInfo};
use super::token::AmazonMusicTokenTracker;
use super::utils::*;

const SEARCH_URL: &str = "https://na.mesk.skill.music.a2z.com/api/showSearch";
const SEARCH_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/144.0.0.0 Safari/537.36";

pub struct AmazonMusicSearch {
    client: Arc<reqwest::Client>,
    token_tracker: Arc<AmazonMusicTokenTracker>,
}

impl AmazonMusicSearch {
    pub fn new(client: Arc<reqwest::Client>, token_tracker: Arc<AmazonMusicTokenTracker>) -> Self {
        Self {
            client,
            token_tracker,
        }
    }

    pub async fn search(&self, query: &str, limit: usize) -> Option<SearchResult> {
        let cfg = match self.token_tracker.get_config().await {
            Some(c) => c,
            None => {
                error!("Amazon Music failed to get config, search aborted.");
                return None;
            }
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_millis();

        let q_enc = urlencoding::encode(query);
        let csrf_header = self.token_tracker.build_csrf_header(&cfg.csrf);

        let search_payload = serde_json::json!({
            "filter": "{\"IsLibrary\":[\"false\"]}",
            "keyword": serde_json::json!({
                "interface": "Web.TemplatesInterface.v1_0.Touch.SearchTemplateInterface.SearchKeywordClientInformation",
                "keyword": ""
            }).to_string(),
            "suggestedKeyword": query,
            "userHash": "{\"level\":\"LIBRARY_MEMBER\"}",
            "headers": build_amazon_headers(
                &cfg,
                now,
                &csrf_header,
                &format!("https://music.amazon.com/search/{}?filter=IsLibrary%7Cfalse&sc=none", q_enc)
            ).to_string()
        });

        let payload_str = search_payload.to_string();

        let res = self.client.post(SEARCH_URL)
            .header("User-Agent", SEARCH_USER_AGENT)
            .header("Content-Type", "text/plain;charset=UTF-8")
            .header("x-amzn-csrf", &cfg.csrf.token)
            .header("Origin", "https://music.amazon.com")
            .header("Referer", "https://music.amazon.com/")
            .body(payload_str)
            .send()
            .await;

        let res: reqwest::Response = match res {
            Ok(r) => r,
            Err(e) => {
                error!("Amazon Music search request failed: {}", e);
                return None;
            }
        };


        if !res.status().is_success() {
            let err_body = res.text().await.ok().unwrap_or_default();
            error!("Amazon Music search error body: {}", err_body);
            return None;
        }

        let data: Value = res.json().await.ok()?;
        let widgets = data["methods"][0]["template"]["widgets"].as_array()?;

        let mut tracks = Vec::new();

        'widget_loop: for widget in widgets {
            let items = match widget["items"].as_array() {
                Some(i) => i,
                None => continue,
            };
            for item in items {
                if tracks.len() >= limit { break 'widget_loop; }
                let label = item["label"].as_str().unwrap_or("");
                let interface = item["interface"].as_str().unwrap_or("");
                let is_song = label == "song";
                let is_square = interface.contains("SquareHorizontalItemElement");

                if !is_song && !is_square {
                    continue;
                }

                let deeplink = item["primaryLink"]["deeplink"].as_str().unwrap_or("");
                let identifier = match crate::sources::amazonmusic::utils::extract_identifier(deeplink) {
                    Some(id) => id,
                    None => continue,
                };
                
                if !is_song && !deeplink.contains("trackAsin=") {
                    continue;
                }

                let title = get_text(&item["primaryText"], "Unknown Track");
                let author = get_text(&item["secondaryText"], "Unknown Artist");
                let artwork_url = item["image"].as_str().map(|s: &str| s.to_string());

                let mut length = 0;
                // Try direct extraction from search results to avoid extra network calls
                for field in ["secondaryText1", "secondaryText2", "tertiaryText"] {
                    if let Some(text) = item[field]["text"].as_str() {
                        if text.contains(':') {
                            length = parse_colon_duration_to_ms(text);
                            if length > 0 { break; }
                        }
                    }
                }

                tracks.push(TrackInfo {
                    identifier: identifier.clone(),
                    is_seekable: true,
                    author,
                    length,
                    is_stream: false,
                    position: 0,
                    title,
                    uri: Some(format!("https://music.amazon.com/tracks/{}", identifier)),
                    artwork_url,
                    isrc: None,
                    source_name: "amazonmusic".to_string(),
                });
            }
        }

        if tracks.is_empty() {
            return None;
        }

        // Fetch durations for top tracks in parallel if still 0. Cap at 5 for latency.
        let fetch_limit = std::cmp::min(tracks.len(), 5);
        let mut fetch_futures = Vec::new();
        let cfg_shared = Arc::new(cfg);

        for i in 0..fetch_limit {
            if tracks[i].length > 0 { continue; }
            let identifier = tracks[i].identifier.clone();
            let client = self.client.clone();
            let token_tracker = self.token_tracker.clone();
            let cfg = cfg_shared.clone();
            fetch_futures.push(async move {
                (i, crate::sources::amazonmusic::metadata::fetch_track_duration_api(&client, &token_tracker, &identifier, Some(cfg)).await)
            });
        }

        let results = futures::future::join_all(fetch_futures).await;

        for (i, duration) in results {
            if let Some(d) = duration {
                tracks[i].length = d;
            }
        }

        let final_tracks = tracks.into_iter().map(crate::protocol::tracks::Track::new).collect();

        Some(SearchResult {
            tracks: final_tracks,
            plugin: serde_json::json!({}),
            ..Default::default()
        })
    }
}
