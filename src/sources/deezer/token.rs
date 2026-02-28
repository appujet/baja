use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use serde_json::Value;
use tracing::{debug, error};

use crate::common::types::Shared;

#[derive(Debug, Clone)]
pub struct DeezerTokens {
    pub session_id: String,
    pub dzr_uniq_id: String,
    pub api_token: String,
    pub license_token: String,
    pub expire_at: Instant,
    pub arl_index: usize,
}

pub struct DeezerTokenTracker {
    client: Arc<reqwest::Client>,
    arls: Vec<String>,
    tokens: Shared<Vec<Option<DeezerTokens>>>,
    current_index: AtomicUsize,
}

impl DeezerTokenTracker {
    pub fn new(client: Arc<reqwest::Client>, arls: Vec<String>) -> Self {
        let size = arls.len();
        Self {
            client,
            arls,
            tokens: Arc::new(tokio::sync::Mutex::new(vec![None; size])),
            current_index: AtomicUsize::new(0),
        }
    }

    pub async fn get_token(&self) -> Option<DeezerTokens> {
        let index = self.current_index.fetch_add(1, Ordering::Relaxed) % self.arls.len();
        self.get_token_at(index).await
    }

    pub async fn get_token_at(&self, index: usize) -> Option<DeezerTokens> {
        {
            let guard = self.tokens.lock().await;
            if let Some(tokens) = &guard[index] {
                if Instant::now() < tokens.expire_at {
                    return Some(tokens.clone());
                }
            }
        }
        self.refresh_session(index).await
    }

    pub async fn invalidate_token(&self, index: usize) {
        let mut guard = self.tokens.lock().await;
        guard[index] = None;
    }

    async fn refresh_session(&self, index: usize) -> Option<DeezerTokens> {
        let arl = &self.arls[index];
        let initial_cookie = format!("arl={}", arl);
        let url = "https://www.deezer.com/ajax/gw-light.php?method=deezer.getUserData&input=3&api_version=1.0&api_token=";
        let req = self.client.get(url).header("Cookie", &initial_cookie);

        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                error!(
                    "DeezerTokenTracker: Failed to refresh session (index {}): {}",
                    index, e
                );
                return None;
            }
        };

        let mut session_id = String::new();
        let mut dzr_uniq_id = String::new();

        for cookie in resp.cookies() {
            match cookie.name() {
                "sid" => session_id = cookie.value().to_string(),
                "dzr_uniq_id" => dzr_uniq_id = cookie.value().to_string(),
                _ => {}
            }
        }

        let body: Value = match resp.json().await {
            Ok(v) => v,
            Err(e) => {
                error!(
                    "DeezerTokenTracker: Failed to parse session response: {}",
                    e
                );
                return None;
            }
        };

        let api_token = body
            .get("results")
            .and_then(|r| r.get("checkForm"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())?;
        let license_token = body
            .get("results")
            .and_then(|r| r.get("USER"))
            .and_then(|u| u.get("OPTIONS"))
            .and_then(|o| o.get("license_token"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_default();

        let tokens = DeezerTokens {
            session_id,
            dzr_uniq_id,
            api_token,
            license_token,
            expire_at: Instant::now() + Duration::from_secs(3600),
            arl_index: index,
        };

        {
            let mut guard = self.tokens.lock().await;
            guard[index] = Some(tokens.clone());
        }

        debug!("DeezerTokenTracker: Refreshed tokens for index {}", index);
        Some(tokens)
    }
}
