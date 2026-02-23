use std::sync::Arc;

use regex::Regex;
use tokio::sync::RwLock;
use tracing::{debug, error};

use crate::common::types::SharedRw;

const EMBED_URL: &str = "https://open.spotify.com/embed/track/4cOdK2wGLETKBW3PvgPWqT";

#[derive(Clone, Debug)]
pub struct SpotifyToken {
    pub access_token: String,
    pub expiry_ms: u64,
}

pub struct SpotifyTokenTracker {
    client: reqwest::Client,
    token: SharedRw<Option<SpotifyToken>>,
    token_regex: Regex,
    expiry_regex: Regex,
}

impl SpotifyTokenTracker {
    pub fn new(client: reqwest::Client) -> Self {
        Self {
            client,
            token: Arc::new(RwLock::new(None)),
            token_regex: Regex::new(r#""accessToken":"([^"]+)""#).unwrap(),
            expiry_regex: Regex::new(r#""accessTokenExpirationTimestampMs":(\d+)"#).unwrap(),
        }
    }

    pub async fn get_token(&self) -> Option<String> {
        {
            let token_lock = self.token.read().await;
            if let Some(token) = &*token_lock {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64;
                // Keep 5-second margin before expiry to account for request time
                if token.expiry_ms > now + 5_000 {
                    return Some(token.access_token.clone());
                }
            }
        }
        self.refresh_token().await
    }

    async fn refresh_token(&self) -> Option<String> {
        debug!("Refreshing Spotify token from embed...");
        let request = self
            .client
            .get(EMBED_URL)
            .header("Accept-Language", "en-US,en;q=0.9")
            .header("Sec-Fetch-Dest", "iframe")
            .header("Sec-Fetch-Mode", "navigate")
            .header("Sec-Fetch-Site", "cross-site");

        let resp = match request.send().await {
            Ok(r) => r,
            Err(e) => {
                error!("Failed to fetch Spotify embed page: {}", e);
                return None;
            }
        };

        if !resp.status().is_success() {
            error!("Embed page returned status {}", resp.status());
            return None;
        }

        let html = match resp.text().await {
            Ok(t) => t,
            Err(e) => {
                error!("Failed to read Spotify embed HTML: {}", e);
                return None;
            }
        };

        let token_caps = self.token_regex.captures(&html);
        let expiry_caps = self.expiry_regex.captures(&html);

        if token_caps.is_none() || expiry_caps.is_none() {
            error!("Token or expiry not found in embed page");
            return None;
        }

        let token = match token_caps.and_then(|c| c.get(1)) {
            Some(m) => m.as_str().to_string(),
            None => {
                error!("Successfully found token caps but group 1 was missing");
                return None;
            }
        };
        let expiry_ms = match expiry_caps.and_then(|c| c.get(1)) {
            Some(m) => m.as_str().parse::<u64>().ok()?,
            None => {
                error!("Successfully found expiry caps but group 1 was missing");
                return None;
            }
        };

        let mut token_lock = self.token.write().await;
        *token_lock = Some(SpotifyToken {
            access_token: token.clone(),
            expiry_ms,
        });

        debug!(
            "Successfully refreshed Spotify token. Expiry: {}",
            expiry_ms
        );
        Some(token)
    }

    pub fn init(self: Arc<Self>) {
        let this = self.clone();
        tokio::spawn(async move {
            this.get_token().await;
        });
    }
}
