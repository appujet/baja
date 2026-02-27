use async_trait::async_trait;
use serde_json::{Value, json};

use super::LyricsProvider;
use crate::protocol::{
    models::{LyricsData, LyricsLine},
    tracks::TrackInfo,
};

const YTM_DOMAIN: &str = "https://music.youtube.com";
const YTM_BASE_API: &str = "https://music.youtube.com/youtubei/v1/";
const YTM_PARAMS: &str = "?alt=json";
const YTM_PARAMS_KEY: &str = "&key=AIzaSyC9XL3ZjWddXya6X74dJoCTL-WEYFDNX30";

pub struct YoutubeMusicLyricsProvider {
    client: reqwest::Client,
}

impl YoutubeMusicLyricsProvider {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
        }
    }

    async fn send_request(&self, endpoint: &str, body: Value, is_mobile: bool) -> Option<Value> {
        let client_name = if is_mobile {
            "ANDROID_MUSIC"
        } else {
            "WEB_REMIX"
        };
        let client_version = if is_mobile {
            "7.21.50"
        } else {
            "1.20240101.01.00"
        };

        let context = json!({
            "context": {
                "client": {
                    "clientName": client_name,
                    "clientVersion": client_version,
                    "hl": "en",
                    "gl": "US",
                },
                "user": {},
            }
        });

        let mut final_body = body.as_object()?.clone();
        for (k, v) in context.as_object()? {
            final_body.insert(k.clone(), v.clone());
        }

        let url = format!(
            "{}{}{}{}",
            YTM_BASE_API, endpoint, YTM_PARAMS, YTM_PARAMS_KEY
        );
        let resp = self.client.post(&url)
            .header("Content-Type", "application/json")
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/115.0.0.0 Safari/537.36")
            .header("Accept-Language", "en-US,en;q=0.9")
            .header("Origin", YTM_DOMAIN)
            .header("Referer", YTM_DOMAIN)
            .header("Cookie", "SOCS=CAI")
            .json(&final_body)
            .send().await.ok()?;

        let json_resp: Value = resp.json().await.ok()?;
        Some(json_resp)
    }

    fn clean(&self, text: &str) -> String {
        let patterns = [
            r#"(?i)\s*\([^)]*(?:official|lyrics?|video|audio|mv|visualizer|color\s*coded|hd|4k|prod\.)[^)]*\)"#,
            r#"(?i)\s*\[[^\]]*(?:official|lyrics?|video|audio|mv|visualizer|color\s*coded|hd|4k|prod\.)[^\]]*\]"#,
            r#"(?i)\s*[([]\s*(?:ft\.?|feat\.?|featuring)\s+[^)\]]+[)\]]"#,
            r#"(?i)\s*-\s*Topic$"#,
            r#"(?i)VEVO$"#,
            r#"(?i)\s*[(\[]\s*Remastered\s*[\)\]]"#,
        ];

        let mut result = text.to_string();
        for pattern in patterns {
            if let Ok(re) = regex::Regex::new(pattern) {
                result = re.replace_all(&result, "").to_string();
            }
        }
        result.trim().to_string()
    }
}

#[async_trait]
impl LyricsProvider for YoutubeMusicLyricsProvider {
    fn name(&self) -> &'static str {
        "youtubemusic"
    }

    async fn load_lyrics(&self, track: &TrackInfo) -> Option<LyricsData> {
        let mut video_id = None;

        if track.source_name == "youtube" || track.source_name == "youtubemusic" {
            video_id = track.uri.as_deref().or(Some(&track.identifier)).map(|s| {
                if s.contains("v=") {
                    s.split("v=")
                        .nth(1)
                        .and_then(|v| v.split('&').next())
                        .unwrap_or(s)
                        .to_string()
                } else if s.contains("youtu.be/") {
                    s.split("youtu.be/")
                        .nth(1)
                        .and_then(|v| v.split('?').next())
                        .unwrap_or(s)
                        .to_string()
                } else {
                    s.to_string()
                }
            });
        }

        if video_id.is_none() {
            let title = self.clean(&track.title);
            let author = self.clean(&track.author);
            let query = format!("{} {}", title, author);

            if let Some(search_results) = self
                .send_request(
                    "search",
                    json!({
                        "query": query,
                        "params": "EgWKAQIIAWoMEA4QChADEAQQCRAF"
                    }),
                    false,
                )
                .await
            {
                if let Some(contents) = search_results
                    .pointer("/contents/sectionListRenderer/contents")
                    .and_then(|v| v.as_array())
                {
                    for section in contents {
                        if let Some(music_shelf) = section.get("musicShelfRenderer") {
                            if let Some(music_contents) =
                                music_shelf.get("contents").and_then(|v| v.as_array())
                            {
                                if !music_contents.is_empty() {
                                    let first_item = &music_contents[0];
                                    if let Some(vid) = first_item.pointer("/musicResponsiveListItemRenderer/playlistItemData/videoId").and_then(|v| v.as_str()) {
                                        video_id = Some(vid.to_string());
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }

                if video_id.is_none() {
                    if let Some(vid) = search_results.pointer("/contents/sectionListRenderer/contents/0/musicCardShelfRenderer/onTap/watchEndpoint/videoId").and_then(|v| v.as_str()) {
                        video_id = Some(vid.to_string());
                    }
                }

                if video_id.is_none() {
                    let search_str = search_results.to_string();
                    if let Some(caps) = regex::Regex::new(r#""videoId":"([^"]+)""#)
                        .unwrap()
                        .captures(&search_str)
                    {
                        video_id = Some(caps[1].to_string());
                    }
                }
            }
        }

        let video_id = video_id?;

        let next_response = self
            .send_request("next", json!({ "videoId": video_id }), false)
            .await?;

        let tabs = next_response.pointer("/contents/singleColumnMusicWatchNextResultsRenderer/tabbedRenderer/watchNextTabbedResultsRenderer/tabs").and_then(|v| v.as_array())?;

        if tabs.len() < 2 {
            return None;
        }

        let browse_id = tabs[1]
            .pointer("/tabRenderer/endpoint/browseEndpoint/browseId")
            .and_then(|v| v.as_str())?;

        // Try extracting timed lyrics
        if let Some(mobile_response) = self
            .send_request("browse", json!({ "browseId": browse_id }), true)
            .await
        {
            if let Some(lyrics_data) = mobile_response.pointer("/contents/elementRenderer/newElement/type/componentType/model/timedLyricsModel/lyricsData/timedLyricsData").and_then(|v| v.as_array()) {
                let mut lines = Vec::new();

                for line in lyrics_data {
                    let text = line["lyricLine"].as_str().unwrap_or("").to_string();
                    let start_time: u64 = line.pointer("/cueRange/startTimeMilliseconds").and_then(|v| v.as_str()).unwrap_or("0").parse().unwrap_or(0);
                    let end_time: u64 = line.pointer("/cueRange/endTimeMilliseconds").and_then(|v| v.as_str()).unwrap_or("0").parse().unwrap_or(0);

                    lines.push(LyricsLine {
                        text,
                        timestamp: start_time,
                        duration: end_time.saturating_sub(start_time),
                    });
                }

                if !lines.is_empty() {
                    return Some(LyricsData {
                        name: track.title.clone(),
                        author: track.author.clone(),
                        provider: "youtubemusic".to_string(),
                        text: lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n"),
                        lines: Some(lines),
                    });
                }
            }
        }

        // Extract description normal lyrics
        if let Some(browse_response) = self
            .send_request("browse", json!({ "browseId": browse_id }), false)
            .await
        {
            if let Some(contents) = browse_response
                .pointer("/contents/sectionListRenderer/contents")
                .and_then(|v| v.as_array())
            {
                if let Some(desc_shelf) = contents
                    .iter()
                    .find_map(|c| c.get("musicDescriptionShelfRenderer"))
                {
                    if let Some(lyrics_text) = desc_shelf
                        .pointer("/description/runs/0/text")
                        .and_then(|v| v.as_str())
                    {
                        let mut lines = Vec::new();
                        for text_line in lyrics_text.split('\n') {
                            let trimmed = text_line.trim();
                            if !trimmed.is_empty() {
                                lines.push(LyricsLine {
                                    text: trimmed.to_string(),
                                    timestamp: 0,
                                    duration: 0,
                                });
                            }
                        }

                        if !lines.is_empty() {
                            return Some(LyricsData {
                                name: track.title.clone(),
                                author: track.author.clone(),
                                provider: "youtubemusic".to_string(),
                                text: lyrics_text.to_string(),
                                lines: Some(lines),
                            });
                        }
                    }
                }
            }
        }

        None
    }
}
