use async_trait::async_trait;
use serde_json::Value;
use regex::Regex;
use crate::api::models::{LyricsData, LyricsLine};
use crate::api::tracks::TrackInfo;
use super::LyricsProvider;

pub struct NeteaseProvider {
    client: reqwest::Client,
    cookies: String,
}

impl NeteaseProvider {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
            cookies: "NMTID=00OAVK3xqDG726ITU6jopU6jF2yMk0AAAGCO8l1BA; JSESSIONID-WYYY=8KQo11YK2GZP45RMlz8Kn80vHZ9%2FGvwzRKQXXy0iQoFKycWdBlQjbfT0MJrFa6hwRfmpfBYKeHliUPH287JC3hNW99WQjrh9b9RmKT%2Fg1Exc2VwHZcsqi7ITxQgfEiee50po28x5xTTZXKoP%2FRMctN2jpDeg57kdZrXz%2FD%2FWghb%5C4DuZ%3A1659124633932; _iuqxldmzr_=32; _ntes_nnid=0db6667097883aa9596ecfe7f188c3ec,1659122833973; _ntes_nuid=0db6667097883aa9596ecfe7f188c3ec; WNMCID=xygast.1659122837568.01.0; WEVNSM=1.0.0; WM_NI=CwbjWAFbcIzPX3dsLP%2F52VB%2Bxr572gmqAYwvN9KU5X5f1nRzBYl0SNf%2BV9FTmmYZy%2FoJLADaZS0Q8TrKfNSBNOt0HLB8rRJh9DsvMOT7%2BCGCQLbvlWAcJBJeXb1P8yZ3RHA%3D; WM_NIKE=9ca17ae2e6ffcda170e2e6ee90c65b85ae87b9aa5483ef8ab3d14a939e9a83c459959caeadce47e991fbaee82af0fea7c3b92a81a9ae8bd64b86beadaaf95c9cedac94cf5cedebfeb7c121bcaefbd8b16dafaf8fbaf67e8ee785b6b854f7baff8fd1728287a4d1d246a6f59adac560afb397bbfc25ad9684a2c76b9a8d00b2bb60b295aaafd24a8e91bcd1cb4882e8beb3c964fb9cbd97d04598e9e5a4c6499394ae97ef5d83bd86a3c96f9cbeffb1bb739aed9ea9c437e2a3; WM_TID=AAkRFnl03RdABEBEQFOBWHCPOeMra4IL; playerid=94262567".to_string(),
        }
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

    fn parse_lrc(&self, lrc: &str) -> Vec<LyricsLine> {
        let mut lines = Vec::new();
        let time_tag_regex = Regex::new(r"\[(\d+):(\d{2})(?:\.(\d{2,3}))?\]").unwrap();

        for raw_line in lrc.lines() {
            let mut times = Vec::new();
            for cap in time_tag_regex.captures_iter(raw_line) {
                let minutes: u64 = cap[1].parse().unwrap_or(0);
                let seconds: u64 = cap[2].parse().unwrap_or(0);
                let ms_str = cap.get(3).map(|m| m.as_str()).unwrap_or("0");
                let mut ms = ms_str.parse::<u64>().unwrap_or(0);
                
                // Pad or truncate ms to 3 digits
                if ms_str.len() == 2 {
                    ms *= 10;
                } else if ms_str.len() > 3 {
                    ms /= 10u64.pow((ms_str.len() - 3) as u32);
                }

                times.push(minutes * 60 * 1000 + seconds * 1000 + ms);
            }

            if times.is_empty() {
                continue;
            }

            let text = time_tag_regex.replace_all(raw_line, "").trim().to_string();
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

    async fn search_track(&self, query: &str) -> Option<Value> {
        let url = format!(
            "https://music.163.com/api/search/pc?limit=10&type=1&offset=0&s={}",
            urlencoding::encode(query)
        );

        let resp = self.client.get(url)
            .header("Cookie", &self.cookies)
            .header("Referer", "https://music.163.com/")
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36")
            .send()
            .await.ok()?;

        let body: Value = resp.json().await.ok()?;
        let songs = body["result"]["songs"].as_array()?;
        
        songs.get(0).cloned()
    }
}

#[async_trait]
impl LyricsProvider for NeteaseProvider {
    fn name(&self) -> &'static str {
        "netease"
    }

    async fn load_lyrics(
        &self,
        track: &TrackInfo,
    ) -> Option<LyricsData> {
        let title = self.clean(&track.title);
        let artist = self.clean(&track.author);
        let query = format!("{} {}", title, artist);

        let song = self.search_track(&query).await?;
        let song_id = song["id"].as_i64()?;
        let song_name = song["name"].as_str().unwrap_or(&track.title).to_string();

        let lyrics_url = format!("https://music.163.com/api/song/lyric?id={}&lv=1", song_id);
        let resp = self.client.get(lyrics_url)
            .header("Cookie", &self.cookies)
            .header("Referer", "https://music.163.com/")
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36")
            .send()
            .await.ok()?;

        let body: Value = resp.json().await.ok()?;
        let raw_lrc = body["lrc"]["lyric"].as_str()?;

        let lines = self.parse_lrc(raw_lrc);
        if lines.is_empty() {
            return Some(LyricsData {
                name: song_name,
                author: artist,
                provider: "netease".to_string(),
                text: raw_lrc.to_string(),
                lines: None,
            });
        }

        let full_text = lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n");

        Some(LyricsData {
            name: song_name,
            author: artist,
            provider: "netease".to_string(),
            text: full_text,
            lines: Some(lines),
        })
    }
}
