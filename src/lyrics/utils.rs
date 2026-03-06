use std::{
    sync::{Arc, LazyLock},
    time::{SystemTime, UNIX_EPOCH},
};

use regex::Regex;
use tokio::sync::RwLock;

use crate::protocol::models::LyricsLine;

static CLEAN_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r#"(?i)\s*\([^)]*(?:official|lyrics?|video|audio|mv|visualizer|color\s*coded|hd|4k|prod\.)[^)]*\)"#,
        r#"(?i)\s*\[[^\]]*(?:official|lyrics?|video|audio|mv|visualizer|color\s*coded|hd|4k|prod\.)[^\]]*\]"#,
        r#"(?i)\s*[([]\s*(?:ft\.?|feat\.?|featuring)\s+[^)\]]+[)\]]"#,
        r#"(?i)\s*-\s*Topic$"#,
        r#"(?i)VEVO$"#,
        r#"(?i)\s*[(\[]\s*Remastered\s*[\)\]]"#,
    ]
    .iter()
    .filter_map(|p| Regex::new(p).ok())
    .collect()
});

static LRC_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\[(\d+):(\d{2})(?:\.(\d{2,3}))?\]"#).unwrap());

pub fn clean_text(text: &str) -> String {
    let mut result = text.to_string();
    for re in CLEAN_PATTERNS.iter() {
        result = re.replace_all(&result, "").to_string();
    }
    result.trim().to_string()
}

pub fn unescape_html(text: &str) -> String {
    text.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
}

pub fn parse_lrc(lrc: &str) -> Vec<LyricsLine> {
    let mut lines = Vec::new();

    for raw_line in lrc.lines() {
        let mut times = Vec::new();
        for cap in LRC_REGEX.captures_iter(raw_line) {
            let minutes: u64 = cap[1].parse().unwrap_or(0);
            let seconds: u64 = cap[2].parse().unwrap_or(0);
            let ms_str = cap.get(3).map_or("0", |m| m.as_str());

            // Pad or truncate ms to 3 digits
            let ms: u64 = if ms_str.len() == 2 {
                ms_str.parse::<u64>().unwrap_or(0) * 10
            } else if ms_str.len() > 3 {
                ms_str[..3].parse().unwrap_or(0)
            } else {
                ms_str.parse().unwrap_or(0)
            };

            times.push(minutes * 60 * 1000 + seconds * 1000 + ms);
        }

        if times.is_empty() {
            continue;
        }

        let text = LRC_REGEX.replace_all(raw_line, "").trim().to_string();
        if text.is_empty() {
            continue;
        }

        for time in times {
            lines.push(LyricsLine {
                text: text.clone(),
                timestamp: time,
                duration: 0,
            });
        }
    }

    lines.sort_by_key(|l| l.timestamp);

    // Calculate durations
    for i in 0..lines.len().saturating_sub(1) {
        let next_ts = lines[i + 1].timestamp;
        lines[i].duration = next_ts.saturating_sub(lines[i].timestamp);
    }

    lines
}

pub struct TokenManager {
    token: Arc<RwLock<Option<(String, u64)>>>,
    ttl_ms: u64,
}

impl TokenManager {
    pub fn new(ttl_ms: u64) -> Self {
        Self {
            token: Arc::new(RwLock::new(None)),
            ttl_ms,
        }
    }

    pub async fn get_token<F, Fut>(&self, fetch_fn: F) -> Option<String>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Option<String>>,
    {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        {
            let lock = self.token.read().await;
            if let Some((token, expiry)) = &*lock
                && now < *expiry
            {
                return Some(token.clone());
            }
        }

        let mut lock = self.token.write().await;
        // Double check
        if let Some((token, expiry)) = &*lock
            && now < *expiry
        {
            return Some(token.clone());
        }

        if let Some(token) = fetch_fn().await {
            *lock = Some((token.clone(), now + self.ttl_ms));
            return Some(token);
        }

        None
    }
}

impl Default for TokenManager {
    fn default() -> Self {
        Self::new(3600 * 1000) // Default 1 hour TTL
    }
}
