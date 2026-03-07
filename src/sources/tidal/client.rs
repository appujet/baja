use std::sync::Arc;

use serde_json::Value;

use super::{
    error::{TidalError, TidalResult},
    token::TidalTokenTracker,
};

const API_BASE: &str = "https://api.tidal.com/v1";

pub struct TidalClient {
    pub inner: Arc<reqwest::Client>,
    pub token_tracker: Arc<TidalTokenTracker>,
    pub country_code: String,
    pub quality: String,
}

impl TidalClient {
    pub fn new(
        inner: Arc<reqwest::Client>,
        token_tracker: Arc<TidalTokenTracker>,
        country_code: String,
        quality: String,
    ) -> Self {
        Self {
            inner,
            token_tracker,
            country_code,
            quality,
        }
    }

    pub fn base_request(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        builder
            .header(
                reqwest::header::USER_AGENT,
                "TIDAL/3704 CFNetwork/1220.1 Darwin/20.3.0",
            )
            .header("Accept-Language", "en-US")
    }

    pub async fn get_json(&self, path: &str) -> TidalResult<Value> {
        let url = if path.starts_with("http") {
            path.to_owned()
        } else {
            format!("{API_BASE}{path}")
        };

        // Append country code if not present
        let url = if !url.contains("countryCode=") {
            if url.contains('?') {
                format!("{url}&countryCode={}", self.country_code)
            } else {
                format!("{url}?countryCode={}", self.country_code)
            }
        } else {
            url
        };

        let mut req = self.base_request(self.inner.get(&url));

        if let Some(t) = self.token_tracker.get_scraper_token().await {
            req = req.header("x-tidal-token", t);
        } else if let Some(t) = self.token_tracker.get_oauth_token().await {
            req = req.header("Authorization", format!("Bearer {t}"));
        } else {
            return Err(TidalError::NoToken);
        }

        let resp = req.send().await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(TidalError::ApiError { status, body });
        }

        Ok(resp.json().await?)
    }
}
