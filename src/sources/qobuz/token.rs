use std::sync::Arc;

use base64::{Engine as _, engine::general_purpose};
use regex::Regex;
use tokio::sync::RwLock;
use tracing::{debug, error, info};

const WEB_PLAYER_BASE_URL: &str = "https://play.qobuz.com";

pub struct QobuzTokens {
    pub app_id: String,
    pub app_secret: String,
    pub user_token: Option<String>,
}

pub struct QobuzTokenTracker {
    client: Arc<reqwest::Client>,
    tokens: Arc<RwLock<Option<QobuzTokens>>>,
    config_user_token: Option<String>,
    config_app_id: Option<String>,
    config_app_secret: Option<String>,
}

impl QobuzTokenTracker {
    pub fn new(
        client: Arc<reqwest::Client>,
        user_token: Option<String>,
        app_id: Option<String>,
        app_secret: Option<String>,
    ) -> Self {
        Self {
            client,
            tokens: Arc::new(RwLock::new(None)),
            config_user_token: user_token,
            config_app_id: app_id,
            config_app_secret: app_secret,
        }
    }

    pub async fn get_tokens(&self) -> Option<Arc<QobuzTokens>> {
        {
            let tokens = self.tokens.read().await;
            if let Some(t) = &*tokens {
                return Some(Arc::new(QobuzTokens {
                    app_id: t.app_id.clone(),
                    app_secret: t.app_secret.clone(),
                    user_token: t.user_token.clone(),
                }));
            }
        }

        self.refresh_tokens().await
    }

    async fn refresh_tokens(&self) -> Option<Arc<QobuzTokens>> {
        let mut tokens_lock = self.tokens.write().await;

        // Double check after lock
        if let Some(t) = &*tokens_lock {
            return Some(Arc::new(QobuzTokens {
                app_id: t.app_id.clone(),
                app_secret: t.app_secret.clone(),
                user_token: t.user_token.clone(),
            }));
        }

        let app_id;
        let app_secret;

        if let (Some(id), Some(secret)) = (&self.config_app_id, &self.config_app_secret) {
            app_id = id.clone();
            app_secret = secret.clone();
            debug!("Using configured Qobuz app_id and app_secret");
        } else {
            debug!("Fetching Qobuz bundle.js for credential extraction...");
            match self.fetch_credentials_from_web().await {
                Ok((id, secret)) => {
                    app_id = id;
                    app_secret = secret;
                    info!("Successfully extracted Qobuz credentials: appId={}", app_id);
                }
                Err(e) => {
                    error!("Failed to extract Qobuz credentials: {}", e);
                    return None;
                }
            }
        }

        let new_tokens = QobuzTokens {
            app_id,
            app_secret,
            user_token: self.config_user_token.clone(),
        };

        let arc_tokens = Arc::new(QobuzTokens {
            app_id: new_tokens.app_id.clone(),
            app_secret: new_tokens.app_secret.clone(),
            user_token: new_tokens.user_token.clone(),
        });

        *tokens_lock = Some(new_tokens);
        Some(arc_tokens)
    }

    async fn fetch_credentials_from_web(&self) -> Result<(String, String), String> {
        let login_page = self
            .client
            .get(format!("{}/login", WEB_PLAYER_BASE_URL))
            .send()
            .await
            .map_err(|e| e.to_string())?
            .text()
            .await
            .map_err(|e| e.to_string())?;

        let bundle_regex =
            Regex::new(r#"<script src="(/resources/\d+\.\d+\.\d+-[a-z]\d{3}/bundle\.js)""#)
                .unwrap();
        let bundle_path = bundle_regex
            .captures(&login_page)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str())
            .ok_or_else(|| "Failed to find bundle.js path in Qobuz login page".to_string())?;

        let bundle_js = self
            .client
            .get(format!("{}{}", WEB_PLAYER_BASE_URL, bundle_path))
            .send()
            .await
            .map_err(|e| e.to_string())?
            .text()
            .await
            .map_err(|e| e.to_string())?;

        let app_id_regex = Regex::new(r#"production:\{api:\{appId:"(.*?)""#).unwrap();
        let app_id = app_id_regex
            .captures(&bundle_js)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().to_string())
            .ok_or_else(|| "Failed to extract appId from bundle.js".to_string())?;

        let seed_regex =
            Regex::new(r#"\):[a-z]\.initialSeed\("(.*?)",window\.utimezone\.(.*?)\)"#).unwrap();
        let seed_captures = seed_regex
            .captures(&bundle_js)
            .ok_or_else(|| "Failed to extract seed and timezone from bundle.js".to_string())?;

        let seed = seed_captures.get(1).unwrap().as_str();
        let timezone_raw = seed_captures.get(2).unwrap().as_str();
        let timezone = format!(
            "{}{}",
            &timezone_raw[..1].to_uppercase(),
            &timezone_raw[1..].to_lowercase()
        );

        let info_extras_regex = Regex::new(&format!(
            r#"(?s)timezones:\[.*?name:.*?/{}",info:"(?P<info>.*?)",extras:"(?P<extras>.*?)""#,
            regex::escape(&timezone)
        ))
        .unwrap();
        let info_extras = info_extras_regex
            .captures(&bundle_js)
            .ok_or_else(|| format!("Failed to extract info/extras for timezone {}", timezone))?;

        let info = info_extras.name("info").unwrap().as_str();
        let extras = info_extras.name("extras").unwrap().as_str();

        let encoded = format!("{}{}{}", seed, info, extras);
        let encoded = &encoded[..encoded.len() - 44];

        let decoded = general_purpose::STANDARD
            .decode(encoded)
            .map_err(|e| format!("Failed to decode appSecret: {}", e))?;

        let app_secret =
            String::from_utf8(decoded).map_err(|e| format!("Invalid UTF-8 in appSecret: {}", e))?;

        Ok((app_id, app_secret))
    }

    pub fn init(self: Arc<Self>) {
        let this = self.clone();
        tokio::spawn(async move {
            this.get_tokens().await;
        });
    }
}
