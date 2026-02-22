use std::sync::Arc;
use futures::future::join_all;
use serde_json::{Value, json};
use tokio::sync::Semaphore;
use tracing::warn;
use crate::sources::spotify::token::SpotifyTokenTracker;

const PARTNER_API_URL: &str = "https://api-partner.spotify.com/pathfinder/v2/query";

pub struct SpotifyHelpers;

impl SpotifyHelpers {
  pub fn base62_to_hex(id: &str) -> String {
    const ALPHABET: &str = "0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let mut bn = 0u128;
    for c in id.chars() {
      if let Some(idx) = ALPHABET.find(c) {
        bn = bn.wrapping_mul(62).wrapping_add(idx as u128);
      }
    }
    format!("{:032x}", bn)
  }

  pub async fn partner_api_request(
    client: &reqwest::Client,
    token_tracker: &Arc<SpotifyTokenTracker>,
    operation: &str,
    variables: Value,
    sha256_hash: &str,
  ) -> Option<Value> {
    let token = token_tracker.get_token().await?;

    let body = json!({
        "variables": variables,
        "operationName": operation,
        "extensions": {
            "persistedQuery": {
                "version": 1,
                "sha256Hash": sha256_hash
            }
        }
    });

    let resp = client
      .post(PARTNER_API_URL)
      .bearer_auth(token)
      .header("App-Platform", "WebPlayer")
      .header("Spotify-App-Version", "1.2.81.104.g225ec0e6")
      .json(&body)
      .send()
      .await
      .ok()?;

    if !resp.status().is_success() {
      warn!("Partner API returned {} for {}", resp.status(), operation);
      return None;
    }

    resp.json().await.ok()
  }

  pub async fn fetch_paginated_items(
    client: &reqwest::Client,
    token_tracker: &Arc<SpotifyTokenTracker>,
    operation: &str,
    sha256_hash: &str,
    base_vars: Value,
    items_pointer: &str,
    total_count: u64,
    page_limit: u64,
    concurrency: usize,
  ) -> Vec<Value> {
    let pages_needed = total_count.saturating_sub(page_limit);
    if pages_needed == 0 {
      return Vec::new();
    }

    let offsets: Vec<u64> = (1..=((total_count - 1) / page_limit)).collect();
    let semaphore = Arc::new(Semaphore::new(concurrency));

    let futs: Vec<_> = offsets
      .into_iter()
      .map(|page_idx| {
        let semaphore = semaphore.clone();
        let mut vars = base_vars.clone();
        if let Some(obj) = vars.as_object_mut() {
          obj.insert("offset".to_string(), json!(page_idx * page_limit));
          obj.insert("limit".to_string(), json!(page_limit));
        }

        let op = operation.to_string();
        let h = sha256_hash.to_string();
        let c = client.clone();
        let tt = token_tracker.clone();

        async move {
          let _permit = semaphore.acquire().await.unwrap();
          Self::partner_api_request(&c, &tt, &op, vars, &h).await
        }
      })
      .collect();

    let results = join_all(futs).await;

    results
      .into_iter()
      .flatten()
      .filter_map(|result| {
        result
          .pointer(items_pointer)
          .and_then(|v| v.as_array())
          .cloned()
      })
      .flatten()
      .collect()
  }
}
