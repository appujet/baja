use async_trait::async_trait;
use crate::api::models::{LyricsData, LyricsLine};
use crate::api::tracks::TrackInfo;
use super::LyricsProvider;
use regex::Regex;

pub struct LrcLibProvider {
    client: reqwest::Client,
}

impl LrcLibProvider {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }

    fn clean(&self, text: &str, remove_feat: bool) -> String {
        let mut result = text.to_string();
        
        let patterns = [
            r#"(?i)\s*\([^)]*(?:official|lyrics?|video|audio|mv|visualizer|color\s*coded|hd|4k|prod\.)[^)]*\)"#,
            r#"(?i)\s*\[[^\]]*(?:official|lyrics?|video|audio|mv|visualizer|color\s*coded|hd|4k|prod\.)[^\]]*\]"#,
            r#"(?i)\s*-\s*Topic$"#,
            r#"(?i)VEVO$"#,
        ];

        for pattern in patterns {
            if let Ok(re) = Regex::new(pattern) {
                result = re.replace_all(&result, "").to_string();
            }
        }

        if remove_feat {
            if let Ok(re) = Regex::new(r#"(?i)\s*[([]\s*(?:ft\.?|feat\.?|featuring)\s+[^)\]]+[)\]]"#) {
                result = re.replace_all(&result, "").to_string();
            }
        }

        result.trim().to_string()
    }

    fn parse_lrc(&self, lrc: &str) -> Vec<LyricsLine> {
        let mut lines = Vec::new();
        let re = Regex::new(r#"\[(\d+):(\d{2})(?:\.(\d{2,3}))?\]"#).unwrap();

        for raw_line in lrc.lines() {
            let mut times = Vec::new();
            for cap in re.captures_iter(raw_line) {
                let minutes: u64 = cap[1].parse().unwrap_or(0);
                let seconds: u64 = cap[2].parse().unwrap_or(0);
                let ms_str = cap.get(3).map_or("0", |m| m.as_str());
                let ms_padded = format!("{:0<3}", ms_str);
                let ms: u64 = ms_padded[..3].parse().unwrap_or(0);
                
                times.push(minutes * 60 * 1000 + seconds * 1000 + ms);
            }

            if times.is_empty() { continue; }
            let text = re.replace_all(raw_line, "").trim().to_string();
            if text.is_empty() { continue; }

            for time in times {
                lines.push(LyricsLine {
                    text: text.clone(),
                    timestamp: time,
                    duration: 0,
                });
            }
        }

        lines.sort_by_key(|l| l.timestamp);
        lines
    }

    fn parse_plain(&self, lyrics: &str) -> Vec<LyricsLine> {
        lyrics.lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .map(|text| LyricsLine {
                text: text.to_string(),
                timestamp: 0,
                duration: 0,
            })
            .collect()
    }
}

#[async_trait]
impl LyricsProvider for LrcLibProvider {
    fn name(&self) -> &'static str { "lrclib" }

    async fn load_lyrics(
        &self,
        track: &TrackInfo,
    ) -> Option<LyricsData> {
        let title = self.clean(&track.title, true);
        let author = self.clean(&track.author, false);
        
        let query = format!("{} {}", title, author);
        let url = format!("https://lrclib.net/api/search?q={}", urlencoding::encode(&query));

        let resp = self.client.get(url).send().await.ok()?;
        let results: serde_json::Value = resp.json().await.ok()?;
        
        let results_arr = results.as_array()?;
        if results_arr.is_empty() { return None; }

        let title_lower = title.to_lowercase();
        let author_lower = author.to_lowercase();

        let best_match = results_arr.iter().find(|r| {
            let r_title = self.clean(r["trackName"].as_str().unwrap_or(""), true).to_lowercase();
            let r_author = self.clean(r["artistName"].as_str().unwrap_or(""), false).to_lowercase();
            let instrumental = r["instrumental"].as_bool().unwrap_or(false);
            
            r_title == title_lower && r_author == author_lower && !instrumental
        }).or_else(|| {
            results_arr.iter().find(|r| {
                let r_title = self.clean(r["trackName"].as_str().unwrap_or(""), true).to_lowercase();
                let instrumental = r["instrumental"].as_bool().unwrap_or(false);
                r_title == title_lower && !instrumental
            })
        }).or_else(|| {
            results_arr.iter().find(|r| !r["instrumental"].as_bool().unwrap_or(false))
        })?;

        let mut lines = Vec::new();
        let mut synced = false;

        if let Some(synced_lyrics) = best_match["syncedLyrics"].as_str() {
            lines = self.parse_lrc(synced_lyrics);
            synced = true;
        } else if let Some(plain_lyrics) = best_match["plainLyrics"].as_str() {
            lines = self.parse_plain(plain_lyrics);
        }

        if lines.is_empty() { return None; }

        let full_text = lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n");

        Some(LyricsData {
            name: best_match["trackName"].as_str().unwrap_or(&track.title).to_string(),
            author: best_match["artistName"].as_str().unwrap_or(&track.author).to_string(),
            provider: "lrclib".to_string(),
            text: full_text,
            lines: if synced { Some(lines) } else { None },
        })
    }
}
