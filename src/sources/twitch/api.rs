use std::sync::Arc;

use serde_json::Value;
use tokio::sync::RwLock;
use tracing::{debug, warn};

const TWITCH_GQL: &str = "https://gql.twitch.tv/gql";
const TWITCH_URL: &str = "https://www.twitch.tv";
const BROWSER_UA: &str = "Mozilla/5.0 (X11; Linux x86_64; rv:125.0) Gecko/20100101 Firefox/125.0";

const METADATA_PAYLOAD: &str = r#"{"operationName":"StreamMetadata","query":"query StreamMetadata($channelLogin: String!) { user(login: $channelLogin) { stream { type } lastBroadcast { title } } }","variables":{"channelLogin":"%s"}}"#;
const ACCESS_TOKEN_PAYLOAD: &str = r#"{"operationName":"PlaybackAccessToken_Template","query":"query PlaybackAccessToken_Template($login: String!,$isLive:Boolean!,$vodID:ID!,$isVod:Boolean!,$playerType:String!){streamPlaybackAccessToken(channelName:$login,params:{platform:\"web\",playerBackend:\"mediaplayer\",playerType:$playerType})@include(if:$isLive){value signature __typename}videoPlaybackAccessToken(id:$vodID,params:{platform:\"web\",playerBackend:\"mediaplayer\",playerType:$playerType})@include(if:$isVod){value signature __typename}}","variables":{"isLive":true,"login":"%s","isVod":false,"vodID":"","playerType":"site"}}"#;

pub struct TwitchGqlClient {
    http: Arc<reqwest::Client>,
    client_id: RwLock<Option<String>>,
    device_id: RwLock<Option<String>>,
}

impl TwitchGqlClient {
    pub fn new(http: Arc<reqwest::Client>, pinned_client_id: Option<String>) -> Self {
        Self {
            http,
            client_id: RwLock::new(pinned_client_id),
            device_id: RwLock::new(None),
        }
    }

    pub fn is_initialized(&self) -> bool {
        self.client_id.try_read().map(|g| g.is_some()).unwrap_or(false)
    }

    pub async fn init_request_headers(&self) {
        if self.client_id.read().await.is_some() {
            return;
        }

        let resp = match self
            .http
            .get(TWITCH_URL)
            .header("Accept", "text/html")
            .header("User-Agent", BROWSER_UA)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                warn!("Twitch: failed to fetch main page: {e}");
                return;
            }
        };

        let cookie_headers: Vec<String> = resp
            .headers()
            .get_all("set-cookie")
            .iter()
            .filter_map(|v| v.to_str().ok().map(str::to_owned))
            .collect();

        for cookie in &cookie_headers {
            if cookie.contains("unique_id=") {
                if let Some(id) = extract_between(cookie, "unique_id=", ";") {
                    *self.device_id.write().await = Some(id.trim().to_string());
                    break;
                }
            }
        }

        let body = match resp.text().await {
            Ok(b) => b,
            Err(e) => {
                warn!("Twitch: failed to read main page body: {e}");
                return;
            }
        };

        if let Some(id) = extract_between(&body, "clientId=\"", "\"") {
            debug!("Twitch: initialized client_id from main page");
            *self.client_id.write().await = Some(id.to_string());
        }
    }

    async fn post_raw(&self, body: String) -> Option<Value> {
        let client_id = self.client_id.read().await.clone()?;
        let device_id = self.device_id.read().await.clone();

        let mut req = self
            .http
            .post(TWITCH_GQL)
            .header("Client-ID", client_id)
            .header("Content-Type", "text/plain;charset=UTF-8")
            .body(body);

        if let Some(did) = device_id {
            req = req.header("X-Device-ID", did);
        }

        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                debug!("Twitch GQL send error: {e}");
                return None;
            }
        };

        if !resp.status().is_success() {
            warn!("Twitch GQL HTTP {}", resp.status());
            return None;
        }

        match resp.json::<Value>().await {
            Ok(v) => Some(v),
            Err(e) => {
                debug!("Twitch GQL JSON parse error: {e}");
                None
            }
        }
    }

    pub async fn fetch_stream_channel_info(&self, channel: &str) -> Option<Value> {
        let payload = METADATA_PAYLOAD.replace("%s", channel);
        let body = match self.post_raw(payload).await {
            Some(b) => b,
            None => {
                debug!("Twitch: stream metadata request failed for '{channel}'");
                return None;
            }
        };

        if let Some(errors) = body["errors"].as_array() {
            for e in errors {
                debug!("Twitch GQL error: {}", e["message"].as_str().unwrap_or("unknown"));
            }
            return None;
        }

        Some(body)
    }

    pub async fn fetch_access_token(&self, channel: &str) -> Option<(String, String)> {
        let payload = ACCESS_TOKEN_PAYLOAD.replace("%s", channel);
        let body = match self.post_raw(payload).await {
            Some(b) => b,
            None => {
                debug!("Twitch: access token request failed for '{channel}'");
                return None;
            }
        };

        if let Some(errors) = body["errors"].as_array() {
            for e in errors {
                debug!("Twitch access token GQL error: {}", e["message"].as_str().unwrap_or("unknown"));
            }
            return None;
        }

        let token = &body["data"]["streamPlaybackAccessToken"];
        let value = match token["value"].as_str() {
            Some(v) => v.to_string(),
            None => {
                debug!("Twitch: access token 'value' missing for '{channel}'");
                return None;
            }
        };
        let sig = match token["signature"].as_str() {
            Some(s) => s.to_string(),
            None => {
                debug!("Twitch: access token 'signature' missing for '{channel}'");
                return None;
            }
        };

        Some((value, sig))
    }

    pub async fn fetch_text(&self, url: &str) -> Option<String> {
        let client_id = self.client_id.read().await.clone();
        let device_id = self.device_id.read().await.clone();

        let mut req = self.http.get(url);
        if let Some(id) = client_id {
            req = req.header("Client-ID", id);
        }
        if let Some(did) = device_id {
            req = req.header("X-Device-ID", did);
        }

        match req.send().await {
            Ok(resp) => match resp.text().await {
                Ok(t) => Some(t),
                Err(e) => {
                    debug!("Twitch: failed to read response body from '{url}': {e}");
                    None
                }
            },
            Err(e) => {
                debug!("Twitch: HTTP GET failed for '{url}': {e}");
                None
            }
        }
    }
}

fn extract_between<'a>(src: &'a str, prefix: &str, suffix: &str) -> Option<&'a str> {
    let start = src.find(prefix)? + prefix.len();
    let end = src[start..].find(suffix)? + start;
    Some(&src[start..end])
}
