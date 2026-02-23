use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use serde_json::Value;
use tokio::sync::RwLock;
use tracing::{debug, error, info};

#[derive(Debug, Clone)]
pub struct PandoraTokens {
    pub auth_token: String,
    pub csrf_token_raw: String,
    pub csrf_token_parsed: String,
    pub expires_at: Instant,
}

pub struct PandoraTokenTracker {
    client: reqwest::Client,
    tokens: Arc<RwLock<Option<PandoraTokens>>>,
    csrf_override: Option<String>,
}

impl PandoraTokenTracker {
    pub fn new(client: reqwest::Client, csrf_override: Option<String>) -> Self {
        Self {
            client,
            tokens: Arc::new(RwLock::new(None)),
            csrf_override,
        }
    }

    pub async fn get_tokens(&self) -> Option<PandoraTokens> {
        {
            let tokens = self.tokens.read().await;
            if let Some(t) = &*tokens {
                if t.expires_at > Instant::now() {
                    return Some(t.clone());
                }
            }
        }

        self.refresh_tokens().await
    }

    pub async fn force_refresh(&self) -> Option<PandoraTokens> {
        self.perform_refresh(true).await
    }

    pub async fn refresh_tokens(&self) -> Option<PandoraTokens> {
        self.perform_refresh(false).await
    }

    async fn perform_refresh(&self, force: bool) -> Option<PandoraTokens> {
        let mut tokens_lock = self.tokens.write().await;

        if !force {
            if let Some(t) = &*tokens_lock {
                if t.expires_at > Instant::now() {
                    return Some(t.clone());
                }
            }
        }

        // Check again after acquiring write lock
        if let Some(t) = &*tokens_lock {
            if t.expires_at > Instant::now() {
                return Some(t.clone());
            }
        }

        debug!("Refreshing Pandora tokens...");

        let (csrf_raw, csrf_parsed) = if let Some(csrf) = &self.csrf_override {
            (
                format!("csrftoken={};Path=/;Domain=.pandora.com;Secure", csrf),
                csrf.clone(),
            )
        } else {
            match self.fetch_csrf_token().await {
                Ok(res) => res,
                Err(e) => {
                    error!("Failed to fetch Pandora CSRF token: {}", e);
                    return None;
                }
            }
        };

        let auth_token = match self.perform_anonymous_login(&csrf_raw, &csrf_parsed).await {
            Ok(token) => token,
            Err(e) => {
                error!("Failed to perform Pandora anonymous login: {}", e);
                return None;
            }
        };

        let new_tokens = PandoraTokens {
            auth_token,
            csrf_token_raw: csrf_raw,
            csrf_token_parsed: csrf_parsed,
            // Tokens usually last 24 hours, but we'll refresh every 12 to be safe
            expires_at: Instant::now() + Duration::from_secs(12 * 3600),
        };

        *tokens_lock = Some(new_tokens.clone());
        info!("Successfully refreshed Pandora tokens");
        Some(new_tokens)
    }

    async fn fetch_csrf_token(&self) -> Result<(String, String), String> {
        let resp = self
            .client
            .head("https://www.pandora.com")
            .send()
            .await
            .map_err(|e| e.to_string())?;

        let cookies = resp.headers().get_all(reqwest::header::SET_COOKIE);
        let regex = regex::Regex::new(r"csrftoken=([a-f0-9]{16})").unwrap();
        for cookie in cookies {
            let cookie_str = cookie.to_str().unwrap_or("");
            if cookie_str.starts_with("csrftoken=") {
                let raw = cookie_str.split(';').next().unwrap_or("").to_string();
                if let Some(captures) = regex.captures(&raw) {
                    if let Some(parsed_match) = captures.get(1) {
                        let parsed = parsed_match.as_str().to_string();
                        return Ok((raw, parsed));
                    }
                }
            }
        }

        Err("CSRF token not found in cookies".to_string())
    }

    async fn perform_anonymous_login(
        &self,
        csrf_raw: &str,
        csrf_parsed: &str,
    ) -> Result<String, String> {
        let resp = self
            .client
            .post("https://www.pandora.com/api/v1/auth/anonymousLogin")
            .header("Cookie", csrf_raw)
            .header("X-CsrfToken", csrf_parsed)
            .header("Content-Type", "application/json")
            .header("Accept", "*/*")
            .body("")
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !resp.status().is_success() {
            return Err(format!(
                "Anonymous login failed with status: {}",
                resp.status()
            ));
        }

        let body: Value = resp.json().await.map_err(|e| e.to_string())?;

        if let Some(error_code) = body.get("errorCode") {
            if error_code.as_i64() == Some(0) {
                return Err("Anonymous login returned error code 0".to_string());
            }
        }

        body.get("authToken")
            .and_then(|t| t.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| "Auth token not found in response".to_string())
    }

    pub fn init(self: Arc<Self>) {
        let this = self.clone();
        tokio::spawn(async move {
            this.get_tokens().await;
        });
    }
}
