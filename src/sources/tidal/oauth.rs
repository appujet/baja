use std::{sync::Arc, time::Duration};

use reqwest::Client;
use tokio::sync::RwLock;
use tracing::{error, info};

use super::model::{DeviceAuthResponse, TokenResponse};

const CLIENT_ID: &str = "fX2JxdmntZWK0ixT";
const CLIENT_SECRET: &str = "1Nn9AfDAjxrgJFJbKNWLeAyKGVGmINuXPPLHVXAvxAg=";

pub struct TidalOAuth {
    pub client: Client,
    pub access_token: RwLock<Option<String>>,
    pub refresh_token: RwLock<Option<String>>,
}

impl TidalOAuth {
    pub fn new(refresh_token: Option<String>) -> Self {
        Self {
            client: Client::builder()
                .user_agent("TIDAL/3704 CFNetwork/1220.1 Darwin/20.3.0")
                .build()
                .unwrap_or_default(),
            access_token: RwLock::new(None),
            refresh_token: RwLock::new(refresh_token),
        }
    }

    pub async fn get_access_token(&self) -> Option<String> {
        if let Some(token) = &*self.access_token.read().await {
            return Some(token.clone());
        }

        if self.refresh_oauth_token().await.is_ok() {
            return self.access_token.read().await.clone();
        }

        None
    }

    pub async fn get_refresh_token(&self) -> Option<String> {
        self.refresh_token.read().await.clone()
    }

    pub async fn initialize_access_token(self: Arc<Self>) {
        if self.refresh_token.read().await.is_some() {
            return;
        }

        info!("Starting Tidal device authorization flow...");

        let form = [("client_id", CLIENT_ID), ("scope", "r_usr w_usr w_sub")];

        let resp = match self
            .client
            .post("https://auth.tidal.com/v1/oauth2/device_authorization")
            .form(&form)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                error!("Tidal device authorization request failed: {}", e);
                return;
            }
        };

        if !resp.status().is_success() {
            error!("Tidal device authorization failed: {}", resp.status());
            return;
        }

        let data: DeviceAuthResponse = match resp.json().await {
            Ok(d) => d,
            Err(e) => {
                error!("Failed to parse device auth response: {}", e);
                return;
            }
        };

        crate::log_println!(
            "\n  ┌────────────────────────────────────────────────────────────┐\n  │                     TIDAL OAUTH LOGIN                      │\n  ├────────────────────────────────────────────────────────────┤\n  │ 1. Visit: {:<48} │\n  │ 2. Log in and authorize the application.                   │\n  └────────────────────────────────────────────────────────────┘\n",
            data.verification_uri_complete
        );

        let oauth = self.clone();
        tokio::spawn(async move {
            oauth.poll_token(data.device_code, data.interval).await;
        });
    }

    async fn poll_token(&self, device_code: String, interval: u64) {
        let mut interval_timer = tokio::time::interval(Duration::from_secs(interval.max(1)));

        loop {
            interval_timer.tick().await;

            let form = [
                ("client_id", CLIENT_ID),
                ("device_code", &device_code),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("scope", "r_usr w_usr w_sub"),
            ];

            let resp = match self
                .client
                .post("https://auth.tidal.com/v1/oauth2/token")
                .basic_auth(CLIENT_ID, Some(CLIENT_SECRET))
                .form(&form)
                .send()
                .await
            {
                Ok(r) => r,
                Err(_) => continue,
            };

            if resp.status().is_success() {
                if let Ok(data) = resp.json::<TokenResponse>().await {
                    let mut at_lock = self.access_token.write().await;
                    let mut rt_lock = self.refresh_token.write().await;

                    *at_lock = Some(data.access_token);
                    *rt_lock = data.refresh_token;

                    info!("Successfully authorized Tidal OAuth");
                    if let Some(ref rt) = *rt_lock {
                        info!("Tidal Refresh Token: {}", rt);
                    }
                    return;
                }
            } else if let Ok(body) = resp.json::<serde_json::Value>().await {
                let error = body["error"].as_str().unwrap_or_default();
                match error {
                    "authorization_pending" => continue,
                    "slow_down" => {
                        interval_timer = tokio::time::interval(Duration::from_secs(interval + 3));
                    }
                    _ => {
                        error!("Tidal OAuth polling failed: {}", error);
                        return;
                    }
                }
            }
        }
    }

    pub async fn refresh_oauth_token(&self) -> Result<(), String> {
        let refresh_token = self.get_refresh_token().await;
        let rt = match refresh_token {
            Some(t) => t,
            None => return Err("No refresh token available".to_string()),
        };

        info!("Refreshing Tidal OAuth token...");

        let form = [
            ("client_id", CLIENT_ID),
            ("refresh_token", &rt),
            ("grant_type", "refresh_token"),
            ("scope", "r_usr w_usr w_sub"),
        ];

        let resp = self
            .client
            .post("https://auth.tidal.com/v1/oauth2/token")
            .basic_auth(CLIENT_ID, Some(CLIENT_SECRET))
            .form(&form)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            error!("Tidal token refresh failed ({}): {}", status, body);
            return Err(format!("Refresh failed: {}", status));
        }

        let data: TokenResponse = resp.json().await.map_err(|e| e.to_string())?;

        let mut at_lock = self.access_token.write().await;
        let mut rt_lock = self.refresh_token.write().await;

        *at_lock = Some(data.access_token);
        if data.refresh_token.is_some() {
            *rt_lock = data.refresh_token;
        }

        info!("Successfully refreshed Tidal OAuth token");
        Ok(())
    }
}
