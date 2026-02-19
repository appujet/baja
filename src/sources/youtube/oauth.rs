use serde_json::{Value, json};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;

const CLIENT_ID: &str = "861556708454-d6dlm3lh05idd8npek18k6be8ba3oc68.apps.googleusercontent.com";
const CLIENT_SECRET: &str = "SboVhoG9s0rNafixCSGGKXAT";

pub struct YouTubeOAuth {
    refresh_tokens: Vec<String>,
    current_token_index: RwLock<usize>,
    access_token: RwLock<Option<String>>,
    token_expiry: RwLock<u64>,
    client: reqwest::Client,
}

impl YouTubeOAuth {
    pub fn new(refresh_tokens: Vec<String>) -> Self {
        Self {
            refresh_tokens,
            current_token_index: RwLock::new(0),
            access_token: RwLock::new(None),
            token_expiry: RwLock::new(0),
            client: reqwest::Client::new(),
        }
    }

    pub async fn get_access_token(&self, idx: usize) -> Option<String> {
        let max_tokens = self.refresh_tokens.len();
        if max_tokens == 0 {
            return None;
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Check if cached token at this index is valid
        {
            let expiry = self.token_expiry.read().await;
            let token = self.access_token.read().await;
            if let Some(t) = token.as_ref() {
                if now < *expiry {
                    return Some(t.clone());
                }
            }
        }

        // Need to refresh using the specified index
        let refresh_token = &self.refresh_tokens[idx % max_tokens];
        if refresh_token.is_empty() {
            return None;
        }

        match self.refresh_token_request(refresh_token).await {
            Ok((new_token, expires_in)) => {
                let mut token_store = self.access_token.write().await;
                let mut expiry_store = self.token_expiry.write().await;
                *token_store = Some(new_token.clone());
                *expiry_store = now + expires_in - 30; // 30s buffer
                return Some(new_token);
            }
            Err(e) => {
                tracing::error!(
                    "Failed to refresh YouTube token for index {}: {}",
                    idx % max_tokens,
                    e
                );
                None
            }
        }
    }

    async fn refresh_token_request(
        &self,
        refresh_token: &str,
    ) -> Result<(String, u64), Box<dyn std::error::Error + Send + Sync>> {
        let res = self
            .client
            .post("https://www.youtube.com/o/oauth2/token")
            .json(&json!({
                "client_id": CLIENT_ID,
                "client_secret": CLIENT_SECRET,
                "refresh_token": refresh_token,
                "grant_type": "refresh_token"
            }))
            .send()
            .await?;

        let status = res.status();
        if status == 200 {
            let body: Value = res.json().await?;
            if let Some(access_token) = body.get("access_token").and_then(|t| t.as_str()) {
                let expires_in = body
                    .get("expires_in")
                    .and_then(|e| e.as_u64())
                    .unwrap_or(3600);
                return Ok((access_token.to_string(), expires_in));
            }
        }

        Err(format!("OAuth refresh failed with status: {}", status).into())
    }

    pub async fn get_auth_header(&self) -> Option<String> {
        if self.refresh_tokens.is_empty() {
            return None;
        }

        let idx = {
            let mut current_idx = self.current_token_index.write().await;
            let val = *current_idx;
            *current_idx = (val + 1) % self.refresh_tokens.len();
            val
        };

        self.get_access_token(idx)
            .await
            .map(|t| format!("Bearer {}", t))
    }
}
