use async_trait::async_trait;
use regex::Regex;

use super::LyricsProvider;
use crate::api::{
    models::{LyricsData, LyricsLine},
    tracks::TrackInfo,
};

pub struct GeniusProvider {
    client: reqwest::Client,
}

impl GeniusProvider {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
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
                result = re.replace_all(&result, "").to_string();
            }
        }
        result.trim().to_string()
    }

    fn unescape_html(&self, text: &str) -> String {
        text.replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&#39;", "'")
            .replace("&apos;", "'")
            .replace("&nbsp;", " ")
    }
}

#[async_trait]
impl LyricsProvider for GeniusProvider {
    fn name(&self) -> &'static str {
        "genius"
    }

    async fn load_lyrics(&self, track: &TrackInfo) -> Option<LyricsData> {
        let title = self.clean(&track.title);
        let author = self.clean(&track.author);

        let query = if title.to_lowercase().starts_with(&author.to_lowercase()) {
            title.clone()
        } else {
            format!("{} {}", title, author)
        };

        let url = format!(
            "https://genius.com/api/search/multi?q={}",
            urlencoding::encode(&query)
        );
        let resp = self.client.get(url).send().await.ok()?;
        let search_data: serde_json::Value = resp.json().await.ok()?;

        let song = search_data["response"]["sections"]
            .as_array()?
            .iter()
            .find(|s| s["type"] == "song")?["hits"]
            .as_array()?
            .get(0)?["result"]
            .clone();

        let song_path = song["path"].as_str()?;
        let song_url = format!("https://genius.com{}", song_path);

        let song_resp = self.client.get(song_url).send().await.ok()?;
        let song_page = song_resp.text().await.ok()?;

        let re =
            Regex::new(r#"(?s)window\.__PRELOADED_STATE__\s*=\s*JSON\.parse\('(.*?)'\);"#).unwrap();
        let caps = re.captures(&song_page)?;
        let lyrics_data_raw = caps.get(1)?.as_str();

        // Unescape any backslash-escaped character, matching NodeLink's replace(/\\(.)/g, '$1')
        let escape_re = Regex::new(r#"\\(.)"#).unwrap();
        let lyrics_data_unescaped = escape_re.replace_all(lyrics_data_raw, "$1");

        let lyrics_json: serde_json::Value = serde_json::from_str(&lyrics_data_unescaped).ok()?;

        let lyrics_content = lyrics_json["songPage"]["lyricsData"]["body"]["html"].as_str()?;

        let tag_re = Regex::new(r#"<[^>]*>"#).unwrap();
        let lyrics_text = lyrics_content
            .replace("<br>", "\n")
            .replace("<br/>", "\n")
            .replace("<br />", "\n");
        let cleaned_lyrics = self.unescape_html(&tag_re.replace_all(&lyrics_text, ""));

        let lines: Vec<LyricsLine> = cleaned_lyrics
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .filter(|l| !(l.starts_with('[') && l.ends_with(']')))
            .map(|l| LyricsLine {
                text: l.to_string(),
                timestamp: 0,
                duration: 0,
            })
            .collect();

        if lines.is_empty() {
            return None;
        }

        let full_text = lines
            .iter()
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        Some(LyricsData {
            name: song["title"].as_str().unwrap_or("original").to_string(),
            author: song["primary_artist"]["name"]
                .as_str()
                .unwrap_or(&track.author)
                .to_string(),
            provider: "genius".to_string(),
            text: full_text,
            lines: None,
        })
    }
}
