use regex::Regex;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info};

#[derive(Clone, Debug)]
pub struct TidalToken {
    pub access_token: String,
    pub expiry_ms: u64,
}

pub struct TidalTokenTracker {
    pub token: RwLock<Option<TidalToken>>,
    pub client: reqwest::Client,
}

impl TidalTokenTracker {
    pub fn new(client: reqwest::Client, initial_token: Option<String>) -> Self {
        let token = if let Some(t) = initial_token {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64;
            Some(TidalToken {
                access_token: t,
                expiry_ms: now + (24 * 60 * 60 * 1000 * 7),
            })
        } else {
            None
        };

        Self {
            token: RwLock::new(token),
            client,
        }
    }

    pub async fn get_token(&self) -> Option<String> {
        {
            let lock = self.token.read().await;
            if let Some(token) = &*lock {
                if self.is_valid(token) {
                    return Some(token.access_token.clone());
                }
            }
        }
        self.refresh_token().await
    }

    fn is_valid(&self, token: &TidalToken) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        token.expiry_ms > now + 10_000
    }

    async fn refresh_token(&self) -> Option<String> {
        info!("Fetching new Tidal API token...");

        let listen_url = "https://listen.tidal.com";
        let resp = match self.client.get(listen_url).send().await {
            Ok(r) => r,
            Err(e) => {
                error!("Failed to fetch Tidal listen page: {}", e);
                return None;
            }
        };

        if !resp.status().is_success() {
            error!("Tidal listen page returned status: {}", resp.status());
            return None;
        }

        let html = resp.text().await.unwrap_or_default();

        // Find src="/assets/index-....js"
        let script_regex = Regex::new(r#"src="(/assets/index-[^"]+\.js)""#).unwrap();
        let script_path = match script_regex.captures(&html) {
            Some(caps) => caps.get(1)?.as_str(),
            None => {
                error!("Could not find index JS in Tidal HTML");
                return None;
            }
        };

        let script_url = format!("https://listen.tidal.com{}", script_path);

        let js_resp = match self.client.get(&script_url).send().await {
            Ok(r) => r,
            Err(e) => {
                error!("Failed to fetch Tidal JS bundle: {}", e);
                return None;
            }
        };

        let js_content = js_resp.text().await.unwrap_or_default();

        // Find clientId:"..." - we want the second one
        let client_id_regex = Regex::new(r#"clientId\s*[:=]\s*"([^"]+)""#).unwrap();
        let mut matches = client_id_regex.captures_iter(&js_content);

        // Skip first match
        matches.next();

        let token_str = match matches.next() {
            Some(caps) => caps.get(1)?.as_str().to_string(),
            None => {
                error!("Could not find second clientId in Tidal JS");
                return None;
            }
        };

        // Cache for 24h (arbitrary, as we don't have expiration from scraper)
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

        info!("Successfully refreshed Tidal token");
        Some(token_str)
    }

    pub fn init(self: Arc<Self>) {
        let this = self.clone();
        tokio::spawn(async move {
            this.get_token().await;
        });
    }
}
