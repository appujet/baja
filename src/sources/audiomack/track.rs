use std::{collections::BTreeMap, net::IpAddr, sync::Arc};

use rand::{Rng, distributions::Alphanumeric, thread_rng};

use crate::sources::{
    audiomack::utils::build_auth_header,
    http::HttpTrack,
    plugin::{DecoderOutput, PlayableTrack},
};

pub struct AudiomackTrack {
    pub stream_url: String,
    pub local_addr: Option<IpAddr>,
}

impl PlayableTrack for AudiomackTrack {
    fn start_decoding(&self, config: crate::config::player::PlayerConfig) -> DecoderOutput {
        let http_track = HttpTrack {
            url: self.stream_url.clone(),
            local_addr: self.local_addr,
            proxy: None,
        };
        http_track.start_decoding(config)
    }
}

pub async fn fetch_stream_url(client: &Arc<reqwest::Client>, identifier: &str) -> Option<String> {
    let nonce = generate_nonce();
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .to_string();

    // Strategy 1: POST /music/{id}/play
    let post_url = format!("https://api.audiomack.com/v1/music/{identifier}/play");
    let mut body = BTreeMap::new();
    body.insert("environment".to_owned(), "desktop-web".to_owned());
    body.insert("session".to_owned(), "backend-session".to_owned());
    body.insert("hq".to_owned(), "true".to_owned());

    let auth_post = build_auth_header("POST", &post_url, &body, &nonce, &timestamp);
    if let Ok(resp) = client
        .post(&post_url)
        .header("Authorization", auth_post)
        .form(&body)
        .send()
        .await
        && let Some(url) = parse_response(resp).await
    {
        return Some(url);
    }

    // Strategy 2: GET /music/play/{id} (fallback)
    let get_url = format!("https://api.audiomack.com/v1/music/play/{identifier}");
    let mut query = BTreeMap::new();
    query.insert("environment".to_owned(), "desktop-web".to_owned());
    query.insert("hq".to_owned(), "true".to_owned());

    let auth_get = build_auth_header("GET", &get_url, &query, &nonce, &timestamp);
    if let Ok(resp) = client
        .get(&get_url)
        .header("Authorization", auth_get)
        .query(&query)
        .send()
        .await
        && let Some(url) = parse_response(resp).await
    {
        return Some(url);
    }

    None
}

async fn parse_response(resp: reqwest::Response) -> Option<String> {
    if !resp.status().is_success() {
        return None;
    }

    let text = resp.text().await.ok()?;

    // Helper to determine if a URL is likely a direct audio stream
    let is_stream = |url: &str| {
        url.contains("music.audiomack.com")
            || url.ends_with(".m4a")
            || url.ends_with(".mp3")
            || url.contains(".m4a?")
            || url.contains(".mp3?")
    };

    if text.starts_with("http") && is_stream(&text) {
        return Some(text);
    }

    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    if let Some(s) = json.as_str()
        && is_stream(s)
    {
        return Some(s.to_owned());
    }

    let results = json.get("results").unwrap_or(&json);
    let potential_url = results
        .get("signedUrl")
        .or_else(|| results.get("signed_url"))
        .or_else(|| results.get("streamUrl"))
        .or_else(|| results.get("stream_url"))
        .or_else(|| results.get("url"))
        .and_then(|v| v.as_str());

    if let Some(url) = potential_url
        && is_stream(url)
    {
        return Some(url.to_owned());
    }

    None
}

fn generate_nonce() -> String {
    thread_rng()
        .sample_iter(&Alphanumeric)
        .take(32)
        .map(char::from)
        .collect()
}
