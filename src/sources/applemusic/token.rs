use std::{
    sync::{Arc, OnceLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use regex::Regex;
use serde_json::Value;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

#[derive(Clone, Debug)]
pub struct AppleMusicToken {
    pub access_token: String,
    pub origin: Option<String>,
    pub expiry_ms: u64,
}

pub struct AppleMusicTokenTracker {
    token: RwLock<Option<AppleMusicToken>>,
    client: Arc<reqwest::Client>,
}

impl AppleMusicTokenTracker {
    pub fn new(client: Arc<reqwest::Client>) -> Self {
        Self {
            token: RwLock::new(None),
            client,
        }
    }

    pub async fn get_token(&self) -> Option<String> {
        {
            let lock = self.token.read().await;
            if let Some(token) = &*lock
                && self.is_valid(token)
            {
                return Some(token.access_token.clone());
            }
        }
        self.refresh_token().await
    }

    pub async fn get_origin(&self) -> Option<String> {
        let lock = self.token.read().await;
        lock.as_ref().and_then(|t| t.origin.clone())
    }

    fn is_valid(&self, token: &AppleMusicToken) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        token.expiry_ms > now + 10_000 // 10 second buffer
    }

    async fn refresh_token(&self) -> Option<String> {
        info!("Fetching new Apple Music API token...");

        let browse_url = "https://music.apple.com";
        let resp = match self.client.get(browse_url).send().await {
            Ok(r) => r,
            Err(e) => {
                error!("Failed to fetch Apple Music root page: {}", e);
                return None;
            }
        };

        if !resp.status().is_success() {
            error!("Apple Music root page returned status: {}", resp.status());
            return None;
        }

        let html = resp.text().await.unwrap_or_default();

        static SCRIPT_REGEX: OnceLock<Regex> = OnceLock::new();
        static INDEX_REGEX: OnceLock<Regex> = OnceLock::new();

        let script_re = SCRIPT_REGEX.get_or_init(|| {
            Regex::new(r#"<script\s+type="module"\s+crossorigin\s+src="(/assets/index[^"]+\.js)""#)
                .unwrap()
        });

        let script_path = match script_re.captures(&html) {
            Some(caps) => caps.get(1).map(|m| m.as_str()),
            None => {
                let index_re =
                    INDEX_REGEX.get_or_init(|| Regex::new(r#"/assets/index[^"]+\.js"#).unwrap());
                index_re.find(&html).map(|m| m.as_str())
            }
        };

        let script_path = match script_path {
            Some(p) => p,
            None => {
                error!("Could not find index JS in Apple Music HTML");
                return None;
            }
        };

        let script_url = if script_path.starts_with("http") {
            script_path.to_owned()
        } else {
            format!("https://music.apple.com{}", script_path)
        };

        let js_resp = match self.client.get(&script_url).send().await {
            Ok(r) => r,
            Err(e) => {
                error!("Failed to fetch Apple Music JS bundle: {}", e);
                return None;
            }
        };

        let js_content = js_resp.text().await.unwrap_or_default();

        static TOKEN_REGEX: OnceLock<Regex> = OnceLock::new();
        let token_re =
            TOKEN_REGEX.get_or_init(|| Regex::new(r#"(ey[\w-]+\.[\w-]+\.[\w-]+)"#).unwrap());

        let token_str = match token_re.find(&js_content) {
            Some(m) => m.as_str().to_owned(),
            None => {
                error!("Could not find bearer token in Apple Music JS");
                return None;
            }
        };

        let (origin, expiry_ms) = self.parse_jwt(&token_str).unwrap_or((None, 0));

        let token = AppleMusicToken {
            access_token: token_str.clone(),
            origin,
            expiry_ms,
        };

        let mut lock = self.token.write().await;
        *lock = Some(token);

        info!("Successfully refreshed Apple Music token");
        Some(token_str)
    }

    fn parse_jwt(&self, token: &str) -> Option<(Option<String>, u64)> {
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() < 2 {
            return None;
        }

        let payload_part = parts[1];
        let decoded = URL_SAFE_NO_PAD.decode(payload_part).ok()?;
        let json: Value = serde_json::from_slice(&decoded).ok()?;

        let origin = json
            .get("root_https_origin")
            .and_then(|v| v.as_array())
            .and_then(|a| a.first())
            .and_then(|v| v.as_str())
            .map(|s| s.to_owned());

        let exp = json
            .get("exp")
            .and_then(|v| v.as_u64())
            .map(|e| e * 1000)
            .unwrap_or(0);

        Some((origin, exp))
    }

    pub fn init(self: Arc<Self>) {
        let this = self.clone();
        tokio::spawn(async move {
            let mut backoff = Duration::from_secs(1);
            loop {
                if this.refresh_token().await.is_some() {
                    break;
                }
                warn!(
                    "Failed to initialize Apple Music token, retrying in {:?}...",
                    backoff
                );
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(60));
            }
        });
    }
}
