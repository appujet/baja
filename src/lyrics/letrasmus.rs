use std::sync::Arc;
use async_trait::async_trait;
use serde_json::Value;
use crate::api::models::{LyricsData, LyricsLine};
use crate::api::tracks::TrackInfo;
use super::LyricsProvider;
use regex::Regex;

pub struct LetrasMusProvider {
    client: reqwest::Client,
}

impl LetrasMusProvider {
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

    fn normalize_lang(&self, lang: &str) -> Option<&'static str> {
        let l = lang.to_lowercase();
        if l.starts_with("pt") { Some("pt") }
        else if l.starts_with("en") { Some("en") }
        else if l.starts_with("es") { Some("es") }
        else { None }
    }
}

#[async_trait]
impl LyricsProvider for LetrasMusProvider {
    fn name(&self) -> &'static str { "letrasmus" }

    async fn load_lyrics(
        &self,
        track: &TrackInfo,
        language: Option<String>,
        _source_manager: Option<Arc<crate::sources::SourceManager>>,
    ) -> Option<LyricsData> {
        let title = self.clean(&track.title);
        let author = self.clean(&track.author);
        let query = format!("{} {}", title, author);

        // Search via Solr API
        let search_url = format!("https://solr.sscdn.co/letras/m1/?q={}&wt=json", urlencoding::encode(&query));
        let resp = self.client.get(search_url).send().await.ok()?;
        let search_data: Value = resp.json().await.ok()?;
        
        let docs = search_data["response"]["docs"].as_array()?;
        let best = docs.iter().find(|d| d["t"] == "2" && d["dns"].is_string() && d["url"].is_string())?;
        
        let dns = best["dns"].as_str()?;
        let url_path = best["url"].as_str()?;
        let page_url = format!("https://www.letras.mus.br/{}/{}/", dns, url_path);

        // Fetch HTML to extract metadata and translation links
        let html = self.client.get(&page_url).send().await.ok()?.text().await.ok()?;
        
        // Extract _omq data (metadata)
        let omq_re = Regex::new(r#"_omq\.push\(\['ui/lyric',\s*(\{[\s\S]*?\})\s*,"#).unwrap();
        let omq = omq_re.captures(&html).and_then(|c| serde_json::from_str::<Value>(c.get(1)?.as_str()).ok());
        
        let letras_id = omq.as_ref().and_then(|o| o["ID"].as_i64());
        let youtube_id = omq.as_ref().and_then(|o| o["YoutubeID"].as_str());

        // Handle translations
        if let Some(lang) = language {
            let norm = self.normalize_lang(&lang);
            if let Some(n) = norm {
                // Check for translations in HTML (window.__translationLanguages)
                let trans_re = Regex::new(r#"window\.__translationLanguages\s*=\s*(\[[\s\S]*?\]);"#).unwrap();
                if let Some(caps) = trans_re.captures(&html) {
                    let trans_list: Value = serde_json::from_str(caps.get(1)?.as_str()).ok()?;
                    if let Some(arr) = trans_list.as_array() {
                        let entry = arr.iter().find(|t| t["languageCode"].as_str().map(|c| c.starts_with(n)).unwrap_or(false));
                        if let Some(e) = entry {
                            let trans_url = format!("https://www.letras.mus.br/{}/{}/{}/", 
                                e["url"]["artist"].as_str()?,
                                e["url"]["song"].as_str()?,
                                e["url"]["translation"].as_str()?
                            );
                            let trans_html = self.client.get(trans_url).send().await.ok()?.text().await.ok()?;
                            
                            // Extract lines from translation page
                            let lyric_re = Regex::new(r#"(?i)<div class="lyric-original[^>]*">([\s\S]*?)</div>"#).unwrap();
                            if let Some(c) = lyric_re.captures(&trans_html) {
                                let content = c.get(1)?.as_str().replace("<br>", "\n").replace("<p>", "").replace("</p>", "\n");
                                let tag_re = Regex::new(r#"<[^>]*>"#).unwrap();
                                let cleaned = tag_re.replace_all(&content, "");
                                let lines: Vec<LyricsLine> = cleaned.lines()
                                    .map(|l| l.trim())
                                    .filter(|l| !l.is_empty())
                                    .map(|l| LyricsLine { text: l.to_string(), timestamp: 0, duration: 0 })
                                    .collect();
                                
                                if !lines.is_empty() {
                                    return Some(LyricsData {
                                        name: omq.as_ref().and_then(|o| o["Name"].as_str()).unwrap_or(&track.title).to_string(),
                                        author: track.author.clone(),
                                        provider: "letrasmus".to_string(),
                                        text: lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n"),
                                        lines: None,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }

        // Try synced lyrics if available
        if let (Some(l_id), Some(y_id)) = (letras_id, youtube_id) {
            let api_url = format!("https://www.letras.mus.br/api/v2/subtitle/{}/{}/", l_id, y_id);
            if let Ok(api_resp) = self.client.get(api_url).send().await {
                if let Ok(api_data) = api_resp.json::<Value>().await {
                    if let Some(sub_str) = api_data["Original"]["Subtitle"].as_str() {
                        if let Ok(parsed_sub) = serde_json::from_str::<Value>(sub_str) {
                            if let Some(sub_arr) = parsed_sub.as_array() {
                                let lines: Vec<LyricsLine> = sub_arr.iter().filter_map(|e| {
                                    let arr = e.as_array()?;
                                    let text = arr.get(0)?.as_str()?;
                                    let start = arr.get(1)?.as_f64()?;
                                    let end = arr.get(2)?.as_f64()?;
                                    Some(LyricsLine {
                                        text: text.to_string(),
                                        timestamp: (start * 1000.0) as u64,
                                        duration: ((end - start) * 1000.0) as u64,
                                    })
                                }).collect();

                                if !lines.is_empty() {
                                    return Some(LyricsData {
                                        name: omq.as_ref().and_then(|o| o["Name"].as_str()).unwrap_or(&track.title).to_string(),
                                        author: track.author.clone(),
                                        provider: "letrasmus".to_string(),
                                        text: lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n"),
                                        lines: Some(lines),
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }

        // Fallback to plain lyrics from HTML
        let lyric_re = Regex::new(r#"(?i)<div class="lyric-original[^>]*">([\s\S]*?)</div>"#).unwrap();
        if let Some(c) = lyric_re.captures(&html) {
            let content = c.get(1)?.as_str().replace("<br>", "\n").replace("<p>", "").replace("</p>", "\n");
            let tag_re = Regex::new(r#"<[^>]*>"#).unwrap();
            let cleaned = tag_re.replace_all(&content, "");
            let lines: Vec<LyricsLine> = cleaned.lines()
                .map(|l| l.trim())
                .filter(|l| !l.is_empty())
                .map(|l| LyricsLine { text: l.to_string(), timestamp: 0, duration: 0 })
                .collect();
            
            if !lines.is_empty() {
                return Some(LyricsData {
                    name: omq.as_ref().and_then(|o| o["Name"].as_str()).unwrap_or(&track.title).to_string(),
                    author: track.author.clone(),
                    provider: "letrasmus".to_string(),
                    text: lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n"),
                    lines: None,
                });
            }
        }

        None
    }
}
