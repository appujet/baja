use std::sync::{Arc, OnceLock};

use regex::Regex;
use tokio::sync::RwLock;
use tracing::{error, info};

use super::{model::TidalToken, oauth::TidalOAuth};

fn script_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"src="(/assets/index-[^"]+\.js)""#).unwrap())
}

fn client_id_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"clientId\s*[:=]\s*"([^"]+)""#).unwrap())
}

pub struct TidalTokenTracker {
    pub token: RwLock<Option<TidalToken>>,
    pub client: Arc<reqwest::Client>,
    pub oauth: Arc<TidalOAuth>,
}

impl TidalTokenTracker {
    pub fn new(client: Arc<reqwest::Client>, oauth: Arc<TidalOAuth>) -> Self {
        Self {
            token: RwLock::new(None),
            client,
            oauth,
        }
    }

    pub async fn get_scraper_token(&self) -> Option<String> {
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

    pub async fn get_oauth_token(&self) -> Option<String> {
        self.oauth.get_access_token().await
    }

    fn is_valid(&self, token: &TidalToken) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        token.expiry_ms > now + 10_000
    }

    async fn refresh_token(&self) -> Option<String> {
        info!("Fetching new Tidal API token via scraper...");

        let listen_url = "https://listen.tidal.com";
        let resp = self.client.get(listen_url).send().await.ok()?;

        if !resp.status().is_success() {
            error!("Tidal listen page returned status: {}", resp.status());
            return None;
        }

        let html = resp.text().await.unwrap_or_default();
        let script_path = script_regex().captures(&html)?.get(1)?.as_str();
        let script_url = format!("https://listen.tidal.com{}", script_path);

        let js_resp = self.client.get(&script_url).send().await.ok()?;
        let js_content = js_resp.text().await.unwrap_or_default();

        let mut matches = client_id_regex().captures_iter(&js_content);
        matches.next(); // Skip first match

        let token_str = matches.next()?.get(1)?.as_str().to_owned();

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        let token = TidalToken {
            access_token: token_str.clone(),
            expiry_ms: now + (24 * 60 * 60 * 1000),
        };

        let mut lock = self.token.write().await;
        *lock = Some(token);

        info!("Successfully refreshed Tidal scraper token");
        Some(token_str)
    }

    pub fn init(self: Arc<Self>) {
        let this = self.clone();
        tokio::spawn(async move {
            this.get_scraper_token().await;
        });
    }

    pub async fn has_oauth_refresh_token(&self) -> bool {
        self.oauth.get_refresh_token().await.is_some()
    }
}
