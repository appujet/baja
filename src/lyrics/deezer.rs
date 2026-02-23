use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::sync::RwLock;
use std::sync::Arc;
use crate::api::models::{LyricsData, LyricsLine};
use crate::api::tracks::TrackInfo;
use crate::configs::HttpProxyConfig;
use super::LyricsProvider;

pub struct DeezerProvider {
    client: reqwest::Client,
    jwt: Arc<RwLock<Option<(String, u64)>>>, // (token, expiry_ms)
}

impl DeezerProvider {
    pub fn new(proxy_config: Option<&HttpProxyConfig>) -> Self {
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
                    tracing::info!("Deezer Lyrics Provider: HTTP Proxy configured");
                } else {
                    tracing::warn!("Deezer Lyrics Provider: Invalid proxy URL: {}", url);
                }
            }
        }

        Self {
            client: client_builder.build().unwrap_or_else(|_| reqwest::Client::new()),
            jwt: Arc::new(RwLock::new(None)),
        }
    }

    async fn get_jwt(&self) -> Option<String> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        {
            let lock = self.jwt.read().await;
            if let Some((token, expiry)) = &*lock {
                if now < *expiry {
                    return Some(token.clone());
                }
            }
        }

        let mut lock = self.jwt.write().await;
        // Double check after write lock
        if let Some((token, expiry)) = &*lock {
            if now < *expiry {
                return Some(token.clone());
            }
        }

        let resp = self.client.get("https://auth.deezer.com/login/anonymous?jo=p&rto=c")
            .send()
            .await.ok()?;
        
        let data: Value = resp.json().await.ok()?;
        let token = data.get("jwt").and_then(|t| t.as_str())?.to_string();
        
        *lock = Some((token.clone(), now + 300_000));
        Some(token)
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

    async fn search_track(&self, title: &str, author: &str) -> Option<String> {
        let query = format!("{} {}", title, author);
        let resp = self.client.get("https://api.deezer.com/2.0/search")
            .query(&[("q", query.as_str()), ("limit", "1")])
            .send()
            .await.ok()?;
        
        let data: Value = resp.json().await.ok()?;
        data.get("data")
            .and_then(|d| d.as_array())
            .and_then(|a| a.get(0))
            .and_then(|t| t.get("id"))
            .map(|id| id.to_string())
    }
}

#[async_trait]
impl LyricsProvider for DeezerProvider {
    fn name(&self) -> &'static str { "deezer" }

    async fn load_lyrics(
        &self,
        track: &TrackInfo,
        _language: Option<String>,
        _source_manager: Option<Arc<crate::sources::SourceManager>>,
    ) -> Option<LyricsData> {
        let jwt = self.get_jwt().await?;

        let title = self.clean(&track.title);
        let author = self.clean(&track.author);

        let track_id = if track.source_name == "deezer" {
            track.identifier.clone()
        } else {
            self.search_track(&title, &author).await?
        };

        let query = r#"
        query GetLyrics($trackId: String!) {
          track(trackId: $trackId) {
            id
            lyrics {
              id
              text
              ...SynchronizedWordByWordLines
              ...SynchronizedLines
              licence
              copyright
              writers
              __typename
            }
            __typename
          }
        }

        fragment SynchronizedWordByWordLines on Lyrics {
          id
          synchronizedWordByWordLines {
            start
            end
            words {
              start
              end
              word
              __typename
            }
            __typename
          }
          __typename
        }

        fragment SynchronizedLines on Lyrics {
          id
          synchronizedLines {
            lrcTimestamp
            line
            lineTranslated
            milliseconds
            duration
            __typename
          }
          __typename
        }
        "#;

        let body = json!({
            "operationName": "GetLyrics",
            "variables": { "trackId": track_id },
            "query": query
        });

        let resp = self.client.post("https://pipe.deezer.com/api")
            .header("Authorization", format!("Bearer {}", jwt))
            .json(&body)
            .send()
            .await.ok()?;
        
        let data: Value = resp.json().await.ok()?;
        let lyrics = data.get("data")
            .and_then(|d| d.get("track"))
            .and_then(|t| t.get("lyrics"))?;

        let mut lines = Vec::new();
        let mut synced = false;

        if let Some(swb) = lyrics.get("synchronizedWordByWordLines").and_then(|l| l.as_array()) {
            if !swb.is_empty() {
                synced = true;
                for line in swb {
                    let start = line.get("start").and_then(|v| v.as_u64()).unwrap_or(0);
                    let end = line.get("end").and_then(|v| v.as_u64()).unwrap_or(0);
                    let words = line.get("words").and_then(|v| v.as_array());
                    
                    let text = words.map(|w| {
                        w.iter().map(|s| s.get("word").and_then(|v| v.as_str()).unwrap_or("")).collect::<Vec<_>>().join(" ")
                    }).unwrap_or_default();

                    lines.push(LyricsLine {
                        text,
                        timestamp: start,
                        duration: end - start,
                    });
                }
            }
        }

        if !synced {
            if let Some(sl) = lyrics.get("synchronizedLines").and_then(|l| l.as_array()) {
                if !sl.is_empty() {
                    synced = true;
                    for line in sl {
                        let text = line.get("line").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        let timestamp = line.get("milliseconds").and_then(|v| v.as_u64()).unwrap_or(0);
                        let duration = line.get("duration").and_then(|v| v.as_u64()).unwrap_or(0);
                        
                        lines.push(LyricsLine {
                            text,
                            timestamp,
                            duration,
                        });
                    }
                }
            }
        }

        let full_text = if let Some(text) = lyrics.get("text").and_then(|v| v.as_str()) {
            text.to_string()
        } else if !lines.is_empty() {
            lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n")
        } else {
            return None;
        };

        Some(LyricsData {
            name: track.title.clone(),
            author: track.author.clone(),
            provider: "deezer".to_string(),
            text: full_text,
            lines: if synced { Some(lines) } else { None },
        })
    }
}
