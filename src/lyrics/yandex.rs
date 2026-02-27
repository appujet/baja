use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose};
use hmac::{Hmac, Mac};
use serde_json::Value;
use sha2::Sha256;

use super::LyricsProvider;
use crate::{
    configs::{HttpProxyConfig, lyrics::YandexLyricsConfig},
    protocol::{
        models::{LyricsData, LyricsLine},
        tracks::TrackInfo,
    },
};

pub struct YandexProvider {
    client: reqwest::Client,
    access_token: Option<String>,
}

impl YandexProvider {
    pub fn new(config: &YandexLyricsConfig, proxy_config: Option<&HttpProxyConfig>) -> Self {
        let mut client_builder = reqwest::Client::builder();

        if let Some(proxy_cfg) = proxy_config {
            if let Some(url) = &proxy_cfg.url {
                if let Ok(mut proxy_obj) = reqwest::Proxy::all(url) {
                    if let Some(user) = &proxy_cfg.username {
                        if let Some(pass) = &proxy_cfg.password {
                            proxy_obj = proxy_obj.basic_auth(user, pass);
                        }
                    }
                    client_builder = client_builder.proxy(proxy_obj);
                    tracing::info!("Yandex Lyrics Provider: HTTP Proxy configured");
                } else {
                    tracing::warn!("Yandex Lyrics Provider: Invalid proxy URL: {}", url);
                }
            }
        }

        Self {
            client: client_builder
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            access_token: config.access_token.clone(),
        }
    }

    fn create_sign(&self, track_id: &str) -> (String, u64) {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let message = format!("{}{}", track_id, ts);
        let mut mac = Hmac::<Sha256>::new_from_slice(b"p93jhgh689SBReK6ghtw62")
            .expect("HMAC can take key of any size");
        mac.update(message.as_bytes());
        let result = mac.finalize();
        let sign = general_purpose::STANDARD.encode(result.into_bytes());
        (urlencoding::encode(&sign).to_string(), ts)
    }

    fn parse_lrc(&self, lrc: &str) -> Vec<LyricsLine> {
        let mut lines = Vec::new();
        let re = regex::Regex::new(r#"\[(\d{2}):(\d{2})\.(\d{2})\]\s*(.*?)(?=\n|\[|$)"#).unwrap();

        for cap in re.captures_iter(lrc) {
            let mins: u64 = cap[1].parse().unwrap_or(0);
            let secs: u64 = cap[2].parse().unwrap_or(0);
            let cs: u64 = cap[3].parse().unwrap_or(0);
            let time = (mins * 60 + secs) * 1000 + cs * 10;
            let text = cap[4].trim().to_string();
            if text.is_empty() {
                continue;
            }
            lines.push(LyricsLine {
                text,
                timestamp: time,
                duration: 0,
            });
        }
        lines
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
impl LyricsProvider for YandexProvider {
    fn name(&self) -> &'static str {
        "yandex"
    }

    async fn load_lyrics(&self, track: &TrackInfo) -> Option<LyricsData> {
        let token = self.access_token.as_ref()?;
        let title = self.clean(&track.title);
        let author = self.clean(&track.author);

        let track_id = if track.source_name == "yandexmusic" {
            track.identifier.clone()
        } else {
            // Search logic if not directly from yandex
            let query = format!("{} {}", title, author);
            let search_url = format!(
                "https://api.music.yandex.net/search?text={}&type=track&page=0",
                urlencoding::encode(&query)
            );
            let search_resp = self
                .client
                .get(search_url)
                .header("Authorization", format!("OAuth {}", token))
                .header("X-Yandex-Music-Client", "YandexMusicAndroid/24023621")
                .send()
                .await
                .ok()?;

            let search_body: Value = search_resp.json().await.ok()?;
            search_body["result"]["tracks"]["results"]
                .as_array()?
                .get(0)?
                .get("id")?
                .as_i64()?
                .to_string()
        };

        let (sign, ts) = self.create_sign(&track_id);
        let url = format!(
            "https://api.music.yandex.net/tracks/{}/lyrics?format=LRC&timeStamp={}&sign={}",
            track_id, ts, sign
        );

        let resp = self
            .client
            .get(url)
            .header("Authorization", format!("OAuth {}", token))
            .header("X-Yandex-Music-Client", "YandexMusicAndroid/24023621")
            .send()
            .await
            .ok()?;

        let body: Value = resp.json().await.ok()?;
        let download_url = body["result"]["downloadUrl"].as_str()?;

        let lrc_resp = self
            .client
            .get(download_url)
            .header("Authorization", format!("OAuth {}", token))
            .send()
            .await
            .ok()?;

        let lrc_text = lrc_resp.text().await.ok()?;
        let lines = self.parse_lrc(&lrc_text);

        if lines.is_empty() {
            return None;
        }

        Some(LyricsData {
            name: track.title.clone(),
            author: track.author.clone(),
            provider: "yandex".to_string(),
            text: lines
                .iter()
                .map(|l| l.text.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            lines: Some(lines),
        })
    }
}
