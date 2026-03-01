use serde_json::Value;
use tracing::{error, warn};

use super::{API_BASE, AppleMusicSource};

impl AppleMusicSource {
    pub(crate) async fn api_request(&self, path: &str) -> Option<Value> {
        let token = self.token_tracker.get_token().await?;
        let origin = self.token_tracker.get_origin().await;

        let url = if path.starts_with("http") {
            path.to_string()
        } else {
            format!("{}{}", API_BASE, path)
        };

        let mut req = self.client.get(&url).bearer_auth(token);

        if let Some(o) = origin {
            req = req.header("Origin", format!("https://{}", o));
        }

        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                error!("Apple Music API request failed: {}", e);
                return None;
            }
        };

        if !resp.status().is_success() {
            warn!("Apple Music API returned {} for {}", resp.status(), url);
            return None;
        }

        resp.json().await.ok()
    }
}
