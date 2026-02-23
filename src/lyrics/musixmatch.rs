use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::RwLock;
use std::sync::Arc;
use crate::api::models::{LyricsData, LyricsLine};
use crate::api::tracks::TrackInfo;
use super::LyricsProvider;
use regex::Regex;

const APP_ID: &str = "web-desktop-app-v1.0";
const TOKEN_TTL: u64 = 55_000;
const DEFAULT_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/142.0.0.0 Safari/537.36";
const DEFAULT_COOKIE: &str = "AWSELB=unknown; x-mxm-user-id=undefined; x-mxm-token-guid=undefined; mxm-encrypted-token=";

pub struct MusixmatchProvider {
    client: reqwest::Client,
    token: Arc<RwLock<Option<(String, u64)>>>,
    guid: String,
}

impl MusixmatchProvider {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .user_agent(DEFAULT_UA)
                .build()
                .unwrap_or_default(),
            token: Arc::new(RwLock::new(None)),
            guid: uuid::Uuid::new_v4().to_string(),
        }
    }

    async fn get_token(&self) -> Option<String> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        {
            let lock = self.token.read().await;
            if let Some((token, expiry)) = &*lock {
                if now < *expiry {
                    return Some(token.clone());
                }
            }
        }

        let mut lock = self.token.write().await;
        // Double check
        if let Some((token, expiry)) = &*lock {
            if now < *expiry {
                return Some(token.clone());
            }
        }

        let resp = self.client.get("https://apic-desktop.musixmatch.com/ws/1.1/token.get")
            .query(&[("app_id", APP_ID)])
            .header("Cookie", DEFAULT_COOKIE)
            .send()
            .await.ok()?;
        
        let body: Value = resp.json().await.ok()?;
        let token = body.get("message")?
            .get("body")?
            .get("user_token")?
            .as_str()?
            .to_string();
        
        *lock = Some((token.clone(), now + TOKEN_TTL));
        Some(token)
    }

    fn clean_title(title: &str) -> String {
        let regex = Regex::new(r"(?i)\s*(\(|\[)(?:official|lyrics?|video|audio|mv|visualizer|color\s*coded|hd|4k|prod\.)[^)\]]*(\)|\])").unwrap();
        let cleaned = regex.replace_all(title, "").to_string();
        cleaned.trim().to_string()
    }
}

#[async_trait]
impl LyricsProvider for MusixmatchProvider {
    fn name(&self) -> &'static str { "musixmatch" }

    async fn load_lyrics(&self, track: &TrackInfo, _language: Option<String>) -> Option<LyricsData> {
        let token = self.get_token().await?;
        let title = Self::clean_title(&track.title);
        let artist = &track.author;

        let resp = self.client.get("https://apic-desktop.musixmatch.com/ws/1.1/macro.subtitles.get")
            .query(&[
                ("format", "json"),
                ("namespace", "lyrics_richsynched"),
                ("subtitle_format", "mxm"),
                ("q_track", &title),
                ("q_artist", artist),
                ("usertoken", &token),
                ("app_id", APP_ID),
                ("guid", &self.guid),
            ])
            .send()
            .await.ok()?;
        
        let body: Value = resp.json().await.ok()?;
        let message = body.get("message")?;
        let header = message.get("header")?;
        
        if header.get("status_code")?.as_i64() != Some(200) {
            return None;
        }

        let calls = message.get("body")?.get("macro_calls")?;
        
        let lyrics_body = calls.get("track.lyrics.get")?
            .get("message")?
            .get("body")?
            .get("lyrics")?
            .get("lyrics_body")?
            .as_str()?;

        let subtitles_body = calls.get("track.subtitles.get")?
            .get("message")?
            .get("body")?
            .get("subtitle_list")?
            .as_array()?
            .get(0)?
            .get("subtitle")?
            .get("subtitle_body")?
            .as_str();

        let mut lines = Vec::new();
        let mut synced = false;

        if let Some(sub_json_str) = subtitles_body {
            if let Ok(sub_data) = serde_json::from_str::<Value>(sub_json_str) {
                if let Some(arr) = sub_data.as_array() {
                    synced = true;
                    for item in arr {
                        let text = item.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        let time_data = item.get("time")?;
                        let total = time_data.get("total").and_then(|v| v.as_f64()).unwrap_or(0.0);
                        let duration = time_data.get("duration").and_then(|v| v.as_f64()).unwrap_or(0.0);
                        
                        lines.push(LyricsLine {
                            text,
                            timestamp: (total * 1000.0) as u64,
                            duration: (duration * 1000.0) as u64,
                        });
                    }
                }
            }
        }

        if !synced {
            // Fallback to splitting plain lyrics if no subtitles found in macro (unlikely for macro but good to have)
            lines = lyrics_body.lines()
                .filter(|l| !l.is_empty())
                .map(|l| LyricsLine { text: l.to_string(), timestamp: 0, duration: 0 })
                .collect();
        }

        Some(LyricsData {
            name: track.title.clone(),
            author: track.author.clone(),
            provider: "musixmatch".to_string(),
            text: lyrics_body.to_string(),
            lines: if synced { Some(lines) } else { None },
        })
    }
}
