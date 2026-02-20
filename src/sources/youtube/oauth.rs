use serde_json::{Value, json};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;
use uuid::Uuid;

const CLIENT_ID: &str = "861556708454-d6dlm3lh05idd8npek18k6be8ba3oc68.apps.googleusercontent.com";
const CLIENT_SECRET: &str = "SboVhoG9s0rNafixCSGGKXAT";
const SCOPES: &str = "http://gdata.youtube.com https://www.googleapis.com/auth/youtube";

pub struct YouTubeOAuth {
    refresh_tokens: RwLock<Vec<String>>,
    current_token_index: RwLock<usize>,
    access_token: RwLock<Option<String>>,
    token_expiry: RwLock<u64>,
    client: reqwest::Client,
}

impl YouTubeOAuth {
    pub fn new(refresh_tokens: Vec<String>) -> Self {
        Self {
            refresh_tokens: RwLock::new(refresh_tokens),
            current_token_index: RwLock::new(0),
            access_token: RwLock::new(None),
            token_expiry: RwLock::new(0),
            client: reqwest::Client::new(),
        }
    }

    pub async fn initialize_access_token(self: std::sync::Arc<Self>) {
        if !self.refresh_tokens.read().await.is_empty() {
            return;
        }

        match self.fetch_device_code().await {
            Ok(response) => {
                let verification_url = response["verification_url"].as_str().unwrap_or_default();
                let user_code = response["user_code"].as_str().unwrap_or_default();
                let device_code = response["device_code"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                let interval = response["interval"].as_u64().unwrap_or(5);

                let inner_width = 60;
                let top_border = format!("  ┌{}┐", "─".repeat(inner_width));
                let sep_border = format!("  ├{}┤", "─".repeat(inner_width));
                let bot_border = format!("  └{}┘", "─".repeat(inner_width));

                // Warning line
                let warning = "!!! USE A BURNER ACCOUNT FOR YOUTUBE OAUTH !!!";
                let warning_pad = inner_width.saturating_sub(warning.len());
                let warning_left = warning_pad / 2;
                let warning_right = warning_pad - warning_left;

                println!("\n\x1b[1;33m{}\x1b[0m", top_border);
                println!(
                    "\x1b[1;33m  │\x1b[0m{:left$}\x1b[1;31m{}\x1b[0m{:right$}\x1b[1;33m│\x1b[0m",
                    "",
                    warning,
                    "",
                    left = warning_left,
                    right = warning_right
                );
                println!("\x1b[1;33m{}\x1b[0m", sep_border);

                // Step 1
                let s1_prefix = " 1. Visit: ";
                let s1_padding =
                    inner_width.saturating_sub(s1_prefix.len() + verification_url.len());
                println!(
                    "\x1b[1;33m  │\x1b[0m\x1b[1;36m{}\x1b[0m\x1b[4;34m{}\x1b[0m{:pad$}\x1b[1;33m│\x1b[0m",
                    s1_prefix,
                    verification_url,
                    "",
                    pad = s1_padding
                );

                // Step 2
                let s2_prefix = " 2. Enter code: ";
                let s2_code = format!(" {} ", user_code);
                let s2_padding = inner_width.saturating_sub(s2_prefix.len() + s2_code.len());
                println!(
                    "\x1b[1;33m  │\x1b[0m\x1b[1;36m{}\x1b[0m\x1b[1;42;30m{}\x1b[0m{:pad$}\x1b[1;33m│\x1b[0m",
                    s2_prefix,
                    s2_code,
                    "",
                    pad = s2_padding
                );

                println!("\x1b[1;33m{}\x1b[0m\n", bot_border);

                let oauth = self.clone();
                tokio::spawn(async move {
                    oauth.poll_for_token(device_code, interval).await;
                });
            }
            Err(e) => {
                tracing::error!("Failed to fetch YouTube device code: {}", e);
            }
        }
    }

    async fn fetch_device_code(&self) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let res = self
            .client
            .post("https://www.youtube.com/o/oauth2/device/code")
            .json(&json!({
                "client_id": CLIENT_ID,
                "scope": SCOPES,
                "device_id": Uuid::new_v4().to_string().replace("-", ""),
                "device_model": "ytlr::"
            }))
            .send()
            .await?;

        Ok(res.json().await?)
    }

    async fn poll_for_token(&self, device_code: String, interval: u64) {
        let mut interval_timer = tokio::time::interval(std::time::Duration::from_secs(interval));
        loop {
            interval_timer.tick().await;

            match self
                .fetch_refresh_token_from_device_code(&device_code)
                .await
            {
                Ok(response) => {
                    if let Some(error) = response["error"].as_str() {
                        match error {
                            "authorization_pending" => continue,
                            "slow_down" => {
                                interval_timer = tokio::time::interval(
                                    std::time::Duration::from_secs(interval + 5),
                                );
                                continue;
                            }
                            "expired_token" => {
                                tracing::error!(
                                    "OAUTH INTEGRATION: The device token has expired. OAuth integration has been canceled."
                                );
                                break;
                            }
                            "access_denied" => {
                                tracing::error!(
                                    "OAUTH INTEGRATION: Account linking was denied. OAuth integration has been canceled."
                                );
                                break;
                            }
                            _ => {
                                tracing::error!("Unhandled OAuth2 error: {}", error);
                                break;
                            }
                        }
                    }

                    if let Some(refresh_token) = response["refresh_token"].as_str() {
                        let mut tokens = self.refresh_tokens.write().await;
                        tokens.push(refresh_token.to_string());
                        println!(
                            "\x1b[1;32mOAUTH INTEGRATION: Token retrieved successfully!\x1b[0m"
                        );
                        println!("\x1b[1;32mRefresh token:\x1b[0m {}", refresh_token);
                        break;
                    }
                }
                Err(e) => {
                    println!(
                        "\x1b[1;31mFailed to fetch YouTube OAuth2 token:\x1b[0m {}",
                        e
                    );
                    break;
                }
            }
        }
    }

    async fn fetch_refresh_token_from_device_code(
        &self,
        device_code: &str,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let res = self
            .client
            .post("https://www.youtube.com/o/oauth2/token")
            .json(&json!({
                "client_id": CLIENT_ID,
                "client_secret": CLIENT_SECRET,
                "code": device_code,
                "grant_type": "http://oauth.net/grant_type/device/1.0"
            }))
            .send()
            .await?;

        Ok(res.json().await?)
    }

    pub async fn get_access_token(&self, idx: usize) -> Option<String> {
        let tokens = self.refresh_tokens.read().await;
        let max_tokens = tokens.len();
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
        let refresh_token = &tokens[idx % max_tokens];
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
        let tokens = self.refresh_tokens.read().await;
        if tokens.is_empty() {
            return None;
        }
        let num_tokens = tokens.len();

        let idx = {
            let mut current_idx = self.current_token_index.write().await;
            let val = *current_idx;
            *current_idx = (val + 1) % num_tokens;
            val
        };

        drop(tokens); // Release read lock before calling get_access_token which may acquire it

        self.get_access_token(idx)
            .await
            .map(|t| format!("Bearer {}", t))
    }
}