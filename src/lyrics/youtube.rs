use std::sync::Arc;
use async_trait::async_trait;
use serde_json::Value;
use crate::api::models::{LyricsData, LyricsLine};
use crate::api::tracks::TrackInfo;
use super::LyricsProvider;

pub struct YoutubeLyricsProvider {
    client: reqwest::Client,
}

impl YoutubeLyricsProvider {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl LyricsProvider for YoutubeLyricsProvider {
    fn name(&self) -> &'static str { "youtube" }

    async fn load_lyrics(
        &self,
        track: &TrackInfo,
        language: Option<String>,
        source_manager: Option<Arc<crate::sources::SourceManager>>,
    ) -> Option<LyricsData> {
        if track.source_name != "youtube" && track.source_name != "ytmusic" {
            return None;
        }

        let captions = if let Some(sm) = source_manager {
            let identifier = track.uri.as_deref().unwrap_or(&track.identifier);
            match sm.load(identifier, None::<Arc<dyn crate::routeplanner::RoutePlanner>>).await {
                crate::api::tracks::LoadResult::Track(t) => {
                    t.plugin_info.get("captions").cloned()
                },
                crate::api::tracks::LoadResult::Search(tracks) => {
                    tracks.first().and_then(|t| t.plugin_info.get("captions")).cloned()
                },
                _ => None
            }
        } else {
            None
        }?;

        let caption_tracks = captions.get("playerCaptionsTracklistRenderer")?
            .get("captionTracks")?
            .as_array()?;

        if caption_tracks.is_empty() { return None; }

        // Find English or requested or first available
        let caption_track = caption_tracks.iter().find(|c| {
            if let Some(lang) = &language {
                c["languageCode"].as_str() == Some(lang)
            } else {
                c["languageCode"].as_str().unwrap_or("").starts_with("en")
            }
        }).or_else(|| caption_tracks.iter().find(|c| c["kind"].as_str() != Some("asr")))
          .or_else(|| caption_tracks.get(0))?;

        let base_url = caption_track["baseUrl"].as_str()?;
        let mut url = if base_url.contains("fmt=") {
            base_url.replace("fmt=json3", "") + "&fmt=json3"
        } else {
            format!("{}&fmt=json3", base_url)
        };

        if let Some(lang) = language {
            if caption_track["languageCode"].as_str() != Some(&lang) && caption_track["isTranslatable"].as_bool() == Some(true) {
                url.push_str(&format!("&tlang={}", lang));
            }
        }

        let lyrics_resp = self.client.get(url).send().await.ok()?;
        let lyrics_json: Value = lyrics_resp.json().await.ok()?;

        let events = lyrics_json["events"].as_array()?;
        let mut lines = Vec::new();

        for event in events {
            let start_ms = event["tStartMs"].as_u64().unwrap_or(0);
            let duration_ms = event["dDurationMs"].as_u64().unwrap_or(0);
            
            let text = event["segs"].as_array().map(|segs| {
                segs.iter().map(|seg| seg["utf8"].as_str().unwrap_or("")).collect::<String>()
            }).unwrap_or_default();

            if text.trim().is_empty() { continue; }

            // Unescape common HTML entities
            let cleaned_text = text.replace("&amp;#39;", "'")
                .replace("&quot;", "\"")
                .replace("&amp;", "&");

            lines.push(LyricsLine {
                text: cleaned_text,
                timestamp: start_ms,
                duration: duration_ms,
            });
        }

        if lines.is_empty() { return None; }

        let full_text = lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n");

        Some(LyricsData {
            name: caption_track["name"]["simpleText"].as_str().unwrap_or("Captions").to_string(),
            author: track.author.clone(),
            provider: "youtube".to_string(),
            text: full_text,
            lines: Some(lines),
        })
    }
}
