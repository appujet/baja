use crate::configs::sources::YouTubeCipherConfig;
use serde_json::{Value, json};
use tokio::sync::RwLock;

pub struct YouTubeCipherManager {
    config: YouTubeCipherConfig,
    client: reqwest::Client,
    sts_cache: RwLock<std::collections::HashMap<String, String>>,
}

impl YouTubeCipherManager {
    pub fn new(config: YouTubeCipherConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
            sts_cache: RwLock::new(std::collections::HashMap::new()),
        }
    }

    pub async fn get_sts(
        &self,
        player_url: &str,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        {
            let cache = self.sts_cache.read().await;
            if let Some(sts) = cache.get(player_url) {
                return Ok(sts.clone());
            }
        }

        let sts = if let Some(url) = &self.config.url {
            let mut headers = reqwest::header::HeaderMap::new();
            if let Some(token) = &self.config.token {
                headers.insert(reqwest::header::AUTHORIZATION, token.parse()?);
            }

            let res = self
                .client
                .post(format!("{}/get_sts", url.trim_end_matches('/')))
                .headers(headers)
                .json(&json!({ "player_url": player_url }))
                .send()
                .await?;

            if res.status() == 200 {
                let body: Value = res.json().await?;
                if let Some(sts) = body.get("sts").and_then(|v| v.as_str()) {
                    sts.to_string()
                } else {
                    return Err("Failed to get STS from remote server".into());
                }
            } else {
                return Err("Failed to get STS from remote server".into());
            }
        } else {
            // Local extraction (fallback or default)
            let res = self.client.get(player_url).send().await?;
            let text = res.text().await?;

            // Regex to find sts or signatureTimestamp
            let re = regex::Regex::new(r#"(?:signatureTimestamp|sts):(\d+)"#)?;
            if let Some(caps) = re.captures(&text) {
                caps[1].to_string()
            } else {
                return Err("Could not find STS in player script".into());
            }
        };

        let mut cache = self.sts_cache.write().await;
        cache.insert(player_url.to_string(), sts.clone());
        Ok(sts)
    }

    pub async fn get_signature_timestamp(
        &self,
    ) -> Result<u32, Box<dyn std::error::Error + Send + Sync>> {
        let player_url =
            "https://www.youtube.com/s/player/6182c448/player_ias.vflset/en_US/base.js";
        let sts = self.get_sts(player_url).await?;
        sts.parse::<u32>().map_err(|e| e.into())
    }

    pub async fn resolve_url(
        &self,
        stream_url: &str,
        player_url: &str,
        n_param: Option<&str>,
        sig: Option<&str>,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let url = self
            .config
            .url
            .as_ref()
            .ok_or("Remote cipher URL not configured")?;

        let mut headers = reqwest::header::HeaderMap::new();
        if let Some(token) = &self.config.token {
            headers.insert(reqwest::header::AUTHORIZATION, token.parse()?);
        }

        let mut body = json!({
            "stream_url": stream_url,
            "player_url": player_url,
        });

        if let Some(n) = n_param {
            body["n_param"] = json!(n);
        }
        if let Some(s) = sig {
            body["encrypted_signature"] = json!(s);
            body["signature_key"] = json!("sig");
        }

        let res = self
            .client
            .post(format!("{}/resolve_url", url.trim_end_matches('/')))
            .headers(headers)
            .json(&body)
            .send()
            .await?;

        let status = res.status();
        if status == 200 {
            let body: Value = res.json().await?;
            if let Some(resolved) = body.get("resolved_url").and_then(|v| v.as_str()) {
                return Ok(resolved.to_string());
            }
            return Err("Resolved URL missing in response".into());
        }

        let err_body = res.text().await?;
        Err(format!("Failed to resolve URL with status {}: {}", status, err_body).into())
    }
}
