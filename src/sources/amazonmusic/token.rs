use std::sync::Arc;
use std::time::{Duration, Instant};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tracing::error;
use tokio::sync::Mutex;

const CONFIG_URL: &str = "https://music.amazon.com/config.json";
const FALLBACK_DEVICE_ID: &str = "13580682033287541";
const FALLBACK_SESSION_ID: &str = "142-4001091-4160417";
const CONFIG_TTL: Duration = Duration::from_secs(60);
const SEARCH_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/144.0.0.0 Safari/537.36";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AmazonCsrf {
    pub token: String,
    pub ts: serde_json::Value,
    pub rnd: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AmazonConfig {
    pub access_token: String,
    pub csrf: AmazonCsrf,
    pub device_id: String,
    pub session_id: String,
    pub version: String,
}

// RawAmazonConfig removed as it was unused

struct Cache {
    config: AmazonConfig,
    expires_at: Instant,
}

pub struct AmazonMusicTokenTracker {
    client: Arc<reqwest::Client>,
    cache: RwLock<Option<Cache>>,
    refresh_lock: Mutex<()>,
}

impl AmazonMusicTokenTracker {
    pub fn new(client: Arc<reqwest::Client>) -> Self {
        Self {
            client,
            cache: RwLock::new(None),
            refresh_lock: Mutex::new(()),
        }
    }

    pub async fn get_config(&self) -> Option<AmazonConfig> {
        {
            let cache = self.cache.read();
            if let Some(c) = &*cache {
                if Instant::now() < c.expires_at {
                    return Some(c.config.clone());
                }
            }
        }

        self.refresh_config().await
    }

    async fn refresh_config(&self) -> Option<AmazonConfig> {
        let _guard = self.refresh_lock.lock().await;

        // Double check cache after acquiring lock
        {
            let cache = self.cache.read();
            if let Some(c) = &*cache {
                if Instant::now() < c.expires_at {
                    return Some(c.config.clone());
                }
            }
        }

        let res = self.client.get(CONFIG_URL)
            .header("User-Agent", SEARCH_USER_AGENT)
            .send()
            .await
            .ok()?;

        if !res.status().is_success() {
            error!("Amazon Music config fetch failed: {}", res.status());
            return None;
        }

        let body = res.text().await.ok().unwrap_or_default();
        let raw: serde_json::Value = serde_json::from_str(&body).map_err(|e| {
            error!("Failed to parse Amazon Music config JSON: {}", e);
            e
        }).ok()?;
        
        // Amazon config can be messy, try multiple potential field names
        let access_token = raw["accessToken"].as_str()
            .or_else(|| raw["access_token"].as_str())
            .unwrap_or("")
            .to_string();

        let csrf_raw = &raw["csrf"];
        let csrf = if !csrf_raw.is_null() {
            AmazonCsrf {
                token: csrf_raw["token"].as_str().unwrap_or_default().to_string(),
                ts: csrf_raw["ts"].clone(),
                rnd: csrf_raw["rnd"].clone(),
            }
        } else {
            error!("Amazon Music config missing CSRF data");
            return None;
        };

        let device_id = raw["deviceId"].as_str()
            .or_else(|| raw["device_id"].as_str())
            .filter(|s| !s.starts_with("000"))
            .map(|s| s.to_string())
            .unwrap_or_else(|| FALLBACK_DEVICE_ID.to_string());

        let session_id = raw["sessionId"].as_str()
            .or_else(|| raw["session_id"].as_str())
            .filter(|s| !s.starts_with("000"))
            .map(|s| s.to_string())
            .unwrap_or_else(|| FALLBACK_SESSION_ID.to_string());

        let version = raw["version"].as_str().unwrap_or("1.0.9527.0").to_string();

        let config = AmazonConfig {
            access_token,
            csrf,
            device_id,
            session_id,
            version,
        };

        let mut cache = self.cache.write();
        *cache = Some(Cache {
            config: config.clone(),
            expires_at: Instant::now() + CONFIG_TTL,
        });

        Some(config)
    }

    pub fn build_csrf_header(&self, csrf: &AmazonCsrf) -> String {
        serde_json::json!({
            "interface": "CSRFInterface.v1_0.CSRFHeaderElement",
            "token": csrf.token,
            "timestamp": csrf.ts,
            "rndNonce": csrf.rnd
        }).to_string()
    }
}
