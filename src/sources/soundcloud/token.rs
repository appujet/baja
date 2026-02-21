use std::{
  sync::Arc,
  time::{Duration, Instant},
};

use regex::Regex;
use tokio::sync::RwLock;
use tracing::{debug, error, info, trace, warn};

use crate::common::types::SharedRw;

const SOUNDCLOUD_URL: &str = "https://soundcloud.com";
const CLIENT_ID_REFRESH_INTERVAL: Duration = Duration::from_secs(3600); // 1 hour

pub struct SoundCloudTokenTracker {
  client: reqwest::Client,
  client_id: SharedRw<CachedClientId>,
  // for extracting client_id from HTML/JS
  asset_re: Regex,
  client_id_re: Regex,
}

struct CachedClientId {
  value: Option<String>,
  last_updated: Option<Instant>,
}

impl CachedClientId {
  fn is_stale(&self) -> bool {
    match self.last_updated {
      None => true,
      Some(t) => t.elapsed() > CLIENT_ID_REFRESH_INTERVAL,
    }
  }
}

impl SoundCloudTokenTracker {
  pub fn new(client: reqwest::Client, config: &crate::configs::SoundCloudConfig) -> Self {
    // Pre-seed client_id from config if provided
    let cached = CachedClientId {
      value: config.client_id.clone(),
      last_updated: if config.client_id.is_some() {
        Some(Instant::now())
      } else {
        None
      },
    };

    Self {
      client,
      client_id: Arc::new(RwLock::new(cached)),
      // asset JS script URLs in the page HTML
      asset_re: Regex::new(r"https://a-v2\.sndcdn\.com/assets/[a-zA-Z0-9_-]+\.js").unwrap(),
      // client_id inside JS scripts
      client_id_re: Regex::new(r#"[^_]client_id[:"=]+\s*"?([a-zA-Z0-9_-]{20,})"?"#).unwrap(),
    }
  }

  pub async fn get_client_id(&self) -> Option<String> {
    {
      let guard = self.client_id.read().await;
      if !guard.is_stale() {
        if let Some(id) = &guard.value {
          return Some(id.clone());
        }
      }
    }

    // Need to refresh
    self.refresh_client_id().await
  }

  pub async fn refresh_client_id(&self) -> Option<String> {
    debug!("Refreshing SoundCloud client_id...");
    trace!("SoundCloud: Fetching client_id from soundcloud.com...");

    let html = match self.client.get(SOUNDCLOUD_URL).send().await {
      Ok(r) => match r.text().await {
        Ok(t) => t,
        Err(e) => {
          error!("SoundCloud: Failed to read main page: {}", e);
          return None;
        }
      },
      Err(e) => {
        error!("SoundCloud: Failed to fetch main page: {}", e);
        return None;
      }
    };

    // Try to find client_id directly in page HTML first
    if let Some(caps) = self.client_id_re.captures(&html) {
      if let Some(m) = caps.get(1) {
        let id = m.as_str().to_string();
        trace!("SoundCloud: Found client_id in main page: {}", id);
        self.store_client_id(id.clone()).await;
        info!("Successfully refreshed SoundCloud client_id");
        return Some(id);
      }
    }

    // Otherwise, find all asset JS URLs and probe them
    let asset_urls: Vec<String> = self
      .asset_re
      .find_iter(&html)
      .map(|m| m.as_str().to_string())
      .collect();

    if asset_urls.is_empty() {
      warn!("SoundCloud: No asset JS URLs found in main page");
      return None;
    }

    trace!(
      "SoundCloud: Found {} asset URLs, probing for client_id",
      asset_urls.len()
    );

    // Try the last few asset scripts (the relevant one is usually one of the last)
    for url in asset_urls.iter().rev().take(9) {
      let js = match self.client.get(url).send().await {
        Ok(r) => match r.text().await {
          Ok(t) => t,
          Err(_) => continue,
        },
        Err(_) => continue,
      };

      if let Some(caps) = self.client_id_re.captures(&js) {
        if let Some(m) = caps.get(1) {
          let id = m.as_str().to_string();
          trace!("SoundCloud: Found client_id in asset {}: {}", url, id);
          self.store_client_id(id.clone()).await;
          info!("Successfully refreshed SoundCloud client_id");
          return Some(id);
        }
      }
    }

    warn!("SoundCloud: client_id not found in any asset scripts");
    None
  }

  pub async fn invalidate(&self) {
    let mut guard = self.client_id.write().await;
    guard.last_updated = None;
  }

  async fn store_client_id(&self, id: String) {
    let mut guard = self.client_id.write().await;
    guard.value = Some(id);
    guard.last_updated = Some(Instant::now());
  }

  pub fn init(self: Arc<Self>) {
    let this = self.clone();
    tokio::spawn(async move {
      this.get_client_id().await;
    });
  }
}
