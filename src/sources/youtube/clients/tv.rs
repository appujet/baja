use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use super::{
  YouTubeClient,
  common::{INNERTUBE_API, make_next_request, resolve_format_url, select_best_audio_format},
};
use crate::{
  api::tracks::Track,
  common::types::AnyResult,
  sources::youtube::{
    cipher::YouTubeCipherManager,
    extractor::{extract_from_next, extract_from_player, extract_track},
    oauth::YouTubeOAuth,
  },
};

const CLIENT_NAME: &str = "TVHTML5";
const CLIENT_ID: &str = "7";
const CLIENT_VERSION: &str = "7.20260113.16.00";
const USER_AGENT: &str = "Mozilla/5.0 (Fuchsia) AppleWebKit/537.36 (KHTML, like Gecko) \
     Chrome/140.0.0.0 Safari/537.36 CrKey/1.56.500000";

pub struct TvClient {
  http: reqwest::Client,
}

impl TvClient {
  pub fn new() -> Self {
    let http = reqwest::Client::builder()
      .user_agent(USER_AGENT)
      .timeout(std::time::Duration::from_secs(10))
      .build()
      .expect("Failed to build TV HTTP client");

    Self { http }
  }

  fn build_context(&self, visitor_data: Option<&str>) -> Value {
    let mut client = json!({
        "clientName": CLIENT_NAME,
        "clientVersion": CLIENT_VERSION,
        "userAgent": USER_AGENT,
        "hl": "en",
        "gl": "US"
    });

    if let Some(vd) = visitor_data {
      if let Some(obj) = client.as_object_mut() {
        obj.insert("visitorData".to_string(), vd.into());
      }
    }

    json!({
        "client": client,
        "user": { "lockedSafetyMode": false },
        "request": { "useSsl": true }
    })
  }

  async fn player_request(
    &self,
    video_id: &str,
    visitor_data: Option<&str>,
    signature_timestamp: Option<u32>,
    oauth: &Arc<YouTubeOAuth>,
  ) -> AnyResult<Value> {
    crate::sources::youtube::clients::common::make_player_request(
      &self.http,
      video_id,
      self.build_context(visitor_data),
      CLIENT_ID,
      CLIENT_VERSION,
      None,
      visitor_data,
      signature_timestamp,
      oauth.get_auth_header().await,
      None,
      None,
      None,
    )
    .await
  }

  async fn next_request(
    &self,
    video_id: Option<&str>,
    playlist_id: Option<&str>,
    visitor_data: Option<&str>,
    oauth: &Arc<YouTubeOAuth>,
  ) -> AnyResult<Value> {
    make_next_request(
      &self.http,
      video_id,
      playlist_id,
      self.build_context(visitor_data),
      CLIENT_ID,
      CLIENT_VERSION,
      visitor_data,
      oauth.get_auth_header().await,
    )
    .await
  }
}

#[async_trait]
impl YouTubeClient for TvClient {
  fn name(&self) -> &str {
    "TV"
  }
  fn client_name(&self) -> &str {
    CLIENT_NAME
  }
  fn client_version(&self) -> &str {
    CLIENT_VERSION
  }
  fn user_agent(&self) -> &str {
    USER_AGENT
  }

  async fn search(
    &self,
    query: &str,
    context: &Value,
    oauth: Arc<YouTubeOAuth>,
  ) -> AnyResult<Vec<Track>> {
    let visitor_data = context
      .get("client")
      .and_then(|c| c.get("visitorData"))
      .and_then(|v| v.as_str())
      .or_else(|| context.get("visitorData").and_then(|v| v.as_str()));

    let body = json!({
        "context": self.build_context(visitor_data),
        "query": query
    });

    let url = format!("{}/youtubei/v1/search?prettyPrint=false", INNERTUBE_API);

    let mut req = self
      .http
      .post(&url)
      .header("X-YouTube-Client-Name", CLIENT_ID)
      .header("X-YouTube-Client-Version", CLIENT_VERSION)
      .header("X-Goog-Api-Format-Version", "2");

    if let Some(vd) = visitor_data {
      req = req.header("X-Goog-Visitor-Id", vd);
    }

    let req = req.json(&body);

    let _ = oauth;

    let res = req.send().await?;
    if !res.status().is_success() {
      return Err(format!("TV search failed: {}", res.status()).into());
    }

    let response: Value = res.json().await?;
    let mut tracks = Vec::new();

    if let Some(sections) = response
      .get("contents")
      .and_then(|c| c.get("sectionListRenderer"))
      .and_then(|s| s.get("contents"))
      .and_then(|c| c.as_array())
    {
      for section in sections {
        if let Some(items) = section
          .get("itemSectionRenderer")
          .and_then(|i| i.get("contents"))
          .and_then(|c| c.as_array())
        {
          for item in items {
            if let Some(track) = extract_track(item, "youtube") {
              tracks.push(track);
            }
          }
        }
      }
    }

    Ok(tracks)
  }

  async fn get_track_info(
    &self,
    track_id: &str,
    context: &Value,
    oauth: Arc<YouTubeOAuth>,
  ) -> AnyResult<Option<Track>> {
    let visitor_data = context
      .get("client")
      .and_then(|c| c.get("visitorData"))
      .and_then(|v| v.as_str())
      .or_else(|| context.get("visitorData").and_then(|v| v.as_str()));

    let body = self
      .player_request(track_id, visitor_data, None, &oauth)
      .await?;

    if body
      .get("playabilityStatus")
      .and_then(|p| p.get("status"))
      .and_then(|s| s.as_str())
      != Some("OK")
    {
      return Ok(None);
    }

    Ok(extract_from_player(&body, "youtube"))
  }

  async fn get_playlist(
    &self,
    playlist_id: &str,
    context: &Value,
    oauth: Arc<YouTubeOAuth>,
  ) -> AnyResult<Option<(Vec<Track>, String)>> {
    let visitor_data = context
      .get("client")
      .and_then(|c| c.get("visitorData"))
      .and_then(|v| v.as_str())
      .or_else(|| context.get("visitorData").and_then(|v| v.as_str()));

    let body = self
      .next_request(None, Some(playlist_id), visitor_data, &oauth)
      .await?;

    Ok(extract_from_next(&body, "youtube"))
  }

  async fn resolve_url(
    &self,
    _url: &str,
    _context: &Value,
    _oauth: Arc<YouTubeOAuth>,
  ) -> AnyResult<Option<Track>> {
    tracing::debug!("{} client does not support resolve_url", self.name());
    Ok(None)
  }

  async fn get_track_url(
    &self,
    track_id: &str,
    context: &Value,
    cipher_manager: Arc<YouTubeCipherManager>,
    oauth: Arc<YouTubeOAuth>,
  ) -> AnyResult<Option<String>> {
    let visitor_data = context
      .get("client")
      .and_then(|c| c.get("visitorData"))
      .and_then(|v| v.as_str())
      .or_else(|| context.get("visitorData").and_then(|v| v.as_str()));

    let signature_timestamp = cipher_manager.get_signature_timestamp().await.ok();
    let body = self
      .player_request(track_id, visitor_data, signature_timestamp, &oauth)
      .await?;

    let playability = body
      .get("playabilityStatus")
      .and_then(|p| p.get("status"))
      .and_then(|s| s.as_str())
      .unwrap_or("UNKNOWN");

    if playability != "OK" {
      tracing::warn!(
        "TV player: video {} not playable (status={})",
        track_id,
        playability
      );
      return Ok(None);
    }

    let streaming_data = match body.get("streamingData") {
      Some(sd) => sd,
      None => {
        tracing::error!("TV player: no streamingData for {}", track_id);
        return Ok(None);
      }
    };

    // TV client gets HLS for live streams
    if let Some(hls) = streaming_data
      .get("hlsManifestUrl")
      .and_then(|v| v.as_str())
    {
      tracing::debug!("TV player: using HLS manifest for {}", track_id);
      return Ok(Some(hls.to_string()));
    }

    let adaptive = streaming_data
      .get("adaptiveFormats")
      .and_then(|v| v.as_array());
    let formats = streaming_data.get("formats").and_then(|v| v.as_array());
    let player_page_url = format!("https://www.youtube.com/watch?v={}", track_id);

    if let Some(best) = select_best_audio_format(adaptive, formats) {
      match resolve_format_url(best, &player_page_url, &cipher_manager).await {
        Ok(Some(url)) => return Ok(Some(url)),
        Ok(None) => {
          tracing::warn!(
            "TV player: best format had no resolvable URL for {}",
            track_id
          );
        }
        Err(e) => {
          tracing::error!(
            "TV player: cipher resolution failed for {}: {}",
            track_id,
            e
          );
          return Err(e);
        }
      }
    }

    Ok(None)
  }
}
