use serde_json::Value;

use super::{DeezerSource, PUBLIC_API_BASE};

impl DeezerSource {
    pub(crate) async fn get_json_public(&self, path: &str) -> Option<Value> {
        let url = format!("{}/{}", PUBLIC_API_BASE, path);
        match self.client.get(&url).send().await {
            Ok(res) => {
                if res.status().is_success() {
                    res.json().await.ok()
                } else {
                    tracing::warn!(
                        "Deezer public API request failed: {} (Status: {})",
                        url,
                        res.status()
                    );
                    None
                }
            }
            Err(e) => {
                tracing::error!(
                    "Deezer public API request error: {} (Error: {}). If this is a connectivity error, check your proxy settings.",
                    url,
                    e
                );
                None
            }
        }
    }
}
