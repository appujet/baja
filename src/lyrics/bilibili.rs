use std::sync::Arc;

use async_trait::async_trait;
use md5::{Digest, Md5};
use regex::Regex;
use serde_json::Value;
use tokio::sync::RwLock;

use super::LyricsProvider;
use crate::protocol::{
    models::{LyricsData, LyricsLine},
    tracks::TrackInfo,
};

const MIXIN_KEY_ENC_TAB: [usize; 64] = [
    46, 47, 18, 2, 53, 8, 23, 32, 15, 50, 10, 31, 58, 3, 45, 35, 27, 43, 5, 49, 33, 9, 42, 19, 29,
    28, 14, 39, 12, 38, 41, 13, 37, 48, 7, 16, 24, 55, 40, 61, 26, 17, 0, 1, 60, 51, 30, 4, 22, 25,
    54, 21, 56, 59, 6, 63, 57, 62, 11, 36, 20, 34, 44, 52,
];

pub struct BilibiliProvider {
    client: reqwest::Client,
    wbi_keys: Arc<RwLock<Option<(String, u64)>>>,
}

impl BilibiliProvider {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
            wbi_keys: Arc::new(RwLock::new(None)),
        }
    }

    async fn get_wbi_keys(&self) -> Option<String> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        {
            let lock = self.wbi_keys.read().await;
            if let Some((keys, expiry)) = &*lock {
                if now < *expiry {
                    return Some(keys.clone());
                }
            }
        }

        let resp = self.client.get("https://api.bilibili.com/x/web-interface/nav")
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36")
            .header("Referer", "https://www.bilibili.com/")
            .send()
            .await.ok()?;

        let body: Value = resp.json().await.ok()?;
        let wbi_img = body.get("data")?.get("wbi_img")?;

        let img_url = wbi_img.get("img_url")?.as_str()?;
        let sub_url = wbi_img.get("sub_url")?.as_str()?;

        let img_key = img_url
            .rsplit('/')
            .next()
            .unwrap_or("")
            .split('.')
            .next()
            .unwrap_or("");
        let sub_key = sub_url
            .rsplit('/')
            .next()
            .unwrap_or("")
            .split('.')
            .next()
            .unwrap_or("");

        let raw_key = format!("{}{}", img_key, sub_key);
        let mut mixin_key = String::new();
        for &index in &MIXIN_KEY_ENC_TAB {
            if let Some(c) = raw_key.chars().nth(index) {
                mixin_key.push(c);
            }
        }

        let final_key = mixin_key[..32].to_string();
        let mut lock = self.wbi_keys.write().await;
        *lock = Some((final_key.clone(), now + 3600 * 1000));
        Some(final_key)
    }

    fn sign_wbi(&self, params: &mut Vec<(String, String)>, mixin_key: &str) -> String {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        params.push(("wts".to_string(), now.to_string()));
        params.sort_by(|a, b| a.0.cmp(&b.0));

        let query = params
            .iter()
            .map(|(k, v)| {
                let v_clean = v.replace(|c: char| "!*()'".contains(c), "");
                format!(
                    "{}={}",
                    urlencoding::encode(k),
                    urlencoding::encode(&v_clean)
                )
            })
            .collect::<Vec<_>>()
            .join("&");

        let mut hasher = Md5::new();
        hasher.update(format!("{}{}", query, mixin_key).as_bytes());
        let w_rid = format!("{:x}", hasher.finalize());

        format!("{}&w_rid={}", query, w_rid)
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
            if let Ok(re) = Regex::new(pattern) {
                result = re.replace_all(&result, "").into_owned();
            }
        }
        result.trim().to_string()
    }
}

#[async_trait]
impl LyricsProvider for BilibiliProvider {
    fn name(&self) -> &'static str {
        "bilibili"
    }

    async fn load_lyrics(&self, track: &TrackInfo) -> Option<LyricsData> {
        if track.source_name != "bilibili" {
            return None;
        }

        let mut bvid = track.identifier.clone();
        if let Some(idx) = bvid.find("?p=") {
            bvid = bvid[..idx].to_string();
        }

        let cid = {
            let url = format!(
                "https://api.bilibili.com/x/web-interface/view?bvid={}",
                bvid
            );
            let resp = self.client.get(&url)
                .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36")
                .header("Referer", "https://www.bilibili.com/")
                .send()
                .await.ok()?;
            let body: Value = resp.json().await.ok()?;
            if body.get("code")?.as_i64() != Some(0) {
                return None;
            }
            let data = body.get("data")?;
            data.get("cid")?.as_u64()?.to_string()
        };

        let mixin_key = self.get_wbi_keys().await?;

        let mut params = vec![("bvid".to_string(), bvid), ("cid".to_string(), cid)];
        let signed_query = self.sign_wbi(&mut params, &mixin_key);

        let url = format!("https://api.bilibili.com/x/player/wbi/v2?{}", signed_query);
        let resp = self.client.get(&url)
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36")
            .header("Referer", "https://www.bilibili.com/")
            .send()
            .await.ok()?;

        let body: Value = resp.json().await.ok()?;
        if body.get("code")?.as_i64() != Some(0) {
            return None;
        }

        let subtitles = body
            .get("data")?
            .get("subtitle")?
            .get("subtitles")?
            .as_array()?;
        if subtitles.is_empty() {
            return None;
        }

        let sub_url = subtitles[0].get("subtitle_url")?.as_str()?;
        let full_url = if sub_url.starts_with("//") {
            format!("https:{}", sub_url)
        } else {
            sub_url.to_string()
        };

        let sub_resp = self.client.get(full_url).send().await.ok()?;
        let sub_data: Value = sub_resp.json().await.ok()?;
        let body_lines = sub_data.get("body")?.as_array()?;

        let mut lines = Vec::new();
        for line in body_lines {
            let from = line.get("from")?.as_f64().unwrap_or(0.0);
            let to = line.get("to")?.as_f64().unwrap_or(0.0);
            let content = line.get("content")?.as_str().unwrap_or("").to_string();

            lines.push(LyricsLine {
                text: content,
                timestamp: (from * 1000.0) as u64,
                duration: ((to - from) * 1000.0) as u64,
            });
        }

        if lines.is_empty() {
            return None;
        }

        let full_text = lines
            .iter()
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        Some(LyricsData {
            name: self.clean(&track.title),
            author: self.clean(&track.author),
            provider: "bilibili".to_string(),
            text: full_text,
            lines: Some(lines),
        })
    }
}
