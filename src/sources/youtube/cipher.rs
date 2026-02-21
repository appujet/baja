use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::sync::RwLock;

use crate::{common::types::AnyResult, configs::sources::YouTubeCipherConfig};

#[derive(Clone)]
pub struct CachedPlayerScript {
  pub url: String,
  pub signature_timestamp: String,
  pub expire_timestamp_ms: Instant,
}

pub struct YouTubeCipherManager {
  config: YouTubeCipherConfig,
  client: reqwest::Client,
  cached_player_script: RwLock<Option<CachedPlayerScript>>,
}

impl YouTubeCipherManager {
  pub fn new(config: YouTubeCipherConfig) -> Self {
    Self {
      config,
      client: reqwest::Client::new(),
      cached_player_script: RwLock::new(None),
    }
  }

  pub async fn get_cached_player_script(&self) -> AnyResult<CachedPlayerScript> {
    {
      let cache = self.cached_player_script.read().await;
      if let Some(script) = &*cache {
        if Instant::now() < script.expire_timestamp_ms {
          return Ok(script.clone());
        }
      }
    }

    let mut cache = self.cached_player_script.write().await;
    // Check again after acquiring write lock
    if let Some(script) = &*cache {
      if Instant::now() < script.expire_timestamp_ms {
        return Ok(script.clone());
      }
    }

    let script = self.get_player_script().await?;
    *cache = Some(script.clone());
    Ok(script)
  }

  async fn get_player_script(&self) -> AnyResult<CachedPlayerScript> {
    let res = self
            .client
            .get("https://www.youtube.com/embed/")
            .header(reqwest::header::USER_AGENT, "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/108.0.0.0 Safari/537.36")
            .send()
            .await?;
    let text = res.text().await?;

    let re = regex::Regex::new(r#""jsUrl":"([^"]+)""#)?;
    let mut script_url = if let Some(caps) = re.captures(&text) {
      caps[1].to_string()
    } else {
      // Fallback to watch page
      let res = self
                .client
                .get("https://www.youtube.com/watch?v=dQw4w9WgXcQ")
                .header(reqwest::header::USER_AGENT, "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/108.0.0.0 Safari/537.36")
                .send()
                .await?;
      let text = res.text().await?;
      if let Some(caps) = re.captures(&text) {
        caps[1].to_string()
      } else {
        return Err("Could not find jsUrl in player script".into());
      }
    };

    // Apply locale replacement like NodeLink does
    let locale_re = regex::Regex::new(r"/([a-z]{2}_[A-Z]{2})/")?;
    script_url = locale_re.replace(&script_url, "/en_US/").to_string();

    let full_url = if script_url.starts_with("http") {
      script_url
    } else {
      format!("https://www.youtube.com{}", script_url)
    };

    let signature_timestamp = self.get_timestamp(&full_url).await?;

    Ok(CachedPlayerScript {
      url: full_url,
      signature_timestamp,
      expire_timestamp_ms: Instant::now() + Duration::from_secs(24 * 60 * 60),
    })
  }

  pub async fn get_timestamp(&self, source_url: &str) -> AnyResult<String> {
    if let Some(url) = &self.config.url {
      let mut headers = reqwest::header::HeaderMap::new();
      if let Some(token) = &self.config.token {
        headers.insert(reqwest::header::AUTHORIZATION, token.parse()?);
      }

      let res = self
        .client
        .post(format!("{}/get_sts", url.trim_end_matches('/')))
        .headers(headers)
        .json(&json!({ "player_url": source_url }))
        .send()
        .await?;

      if res.status() == 200 {
        let body: Value = res.json().await?;
        if let Some(sts) = body.get("sts").and_then(|v| v.as_str()) {
          return Ok(sts.to_string());
        } else {
          return Err("Failed to get STS from remote server".into());
        }
      } else {
        return Err("Failed to get STS from remote server".into());
      }
    }

    // Local extraction (fallback or default)
    let res = self.client.get(source_url).send().await?;
    let text = res.text().await?;

    // Regex to find sts or signatureTimestamp
    let re = regex::Regex::new(r#"(?:signatureTimestamp|sts):(\d+)"#)?;
    if let Some(caps) = re.captures(&text) {
      Ok(caps[1].to_string())
    } else {
      Err("Could not find STS in player script".into())
    }
  }

  pub async fn get_signature_timestamp(&self) -> AnyResult<u32> {
    let script = self.get_cached_player_script().await?;
    script
      .signature_timestamp
      .parse::<u32>()
      .map_err(|e| e.into())
  }

  pub async fn resolve_url(
    &self,
    stream_url: &str,
    player_url: &str, // Kept to satisfy caller signatures
    n_param: Option<&str>,
    sig: Option<&str>,
  ) -> AnyResult<String> {
    let url = self
      .config
      .url
      .as_ref()
      .ok_or("Remote cipher URL not configured")?;

    let player_url = if let Ok(script) = self.get_cached_player_script().await {
      script.url
    } else {
      player_url.to_string()
    };

    let mut headers = reqwest::header::HeaderMap::new();
    if let Some(token) = &self.config.token {
      headers.insert(reqwest::header::AUTHORIZATION, token.parse()?);
    }

    let mut body = json!({
        "stream_url": stream_url,
        "player_url": player_url,
    });

    if let Some(n) = n_param {
      body["n_param"] = json!(n);
    }
    if let Some(s) = sig {
      body["encrypted_signature"] = json!(s);
      body["signature_key"] = json!("sig");
    }

    let res = self
      .client
      .post(format!("{}/resolve_url", url.trim_end_matches('/')))
      .headers(headers)
      .json(&body)
      .send()
      .await?;

    let status = res.status();
    if status == 200 {
      let body: Value = res.json().await?;
      if let Some(resolved) = body.get("resolved_url").and_then(|v| v.as_str()) {
        return Ok(resolved.to_string());
      }
      return Err("Resolved URL missing in response".into());
    }

    let err_body = res.text().await?;
    Err(format!("Failed to resolve URL with status {}: {}", status, err_body).into())
  }
}
