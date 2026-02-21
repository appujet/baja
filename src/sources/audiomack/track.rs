use std::net::IpAddr;
use std::collections::BTreeMap;
use flume::{Receiver, Sender};

use crate::{
    audio::processor::DecoderCommand,
    sources::{
        plugin::PlayableTrack,
        http::HttpTrack,
        audiomack::utils::build_auth_header,
    },
};

pub struct AudiomackTrack {
    pub client: reqwest::Client,
    pub identifier: String,
    pub local_addr: Option<IpAddr>,
}

impl PlayableTrack for AudiomackTrack {
    fn start_decoding(&self) -> (Receiver<i16>, Sender<DecoderCommand>) {
        let (tx, rx) = flume::bounded::<i16>(4096 * 4);
        let (cmd_tx, cmd_rx) = flume::unbounded::<DecoderCommand>();

        let identifier = self.identifier.clone();
        let client = self.client.clone();
        let local_addr = self.local_addr;

        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();

            runtime.block_on(async {
                if let Some(url) = fetch_stream_url(&client, &identifier).await {
                    let http_track = HttpTrack { url, local_addr };
                    let (inner_rx, inner_cmd_tx): (Receiver<i16>, Sender<DecoderCommand>) = http_track.start_decoding();

                    // Proxy commands
                    let cmd_tx_clone: Sender<DecoderCommand> = inner_cmd_tx.clone();
                    std::thread::spawn(move || {
                        while let Ok(cmd) = cmd_rx.recv() {
                            let _ = cmd_tx_clone.send(cmd);
                        }
                    });

                    // Proxy samples
                    while let Ok(sample) = inner_rx.recv() {
                        if tx.send(sample).is_err() {
                            break;
                        }
                    }
                }
            });
        });

        (rx, cmd_tx)
    }
}

async fn fetch_stream_url(client: &reqwest::Client, identifier: &str) -> Option<String> {
    let nonce = thread_rng_nonce();
    let timestamp = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()).to_string();

    // Strategy 1: POST /music/{id}/play
    let post_url = format!("https://api.audiomack.com/v1/music/{}/play", identifier);
    let mut body = BTreeMap::new();
    body.insert("environment".to_string(), "desktop-web".to_string());
    body.insert("session".to_string(), "backend-session".to_string());
    body.insert("hq".to_string(), "true".to_string());

    let auth_post = build_auth_header("POST", &post_url, &body, &nonce, &timestamp);
    if let Ok(resp) = client.post(&post_url).header("Authorization", auth_post).form(&body).send().await {
        if let Some(url) = parse_response(resp).await {
            return Some(url);
        }
    }

    // Strategy 2: GET /music/play/{id}
    let get_url = format!("https://api.audiomack.com/v1/music/play/{}", identifier);
    let mut query = BTreeMap::new();
    query.insert("environment".to_string(), "desktop-web".to_string());
    query.insert("hq".to_string(), "true".to_string());

    let auth_get = build_auth_header("GET", &get_url, &query, &nonce, &timestamp);
    if let Ok(resp) = client.get(&get_url).header("Authorization", auth_get).query(&query).send().await {
        if let Some(url) = parse_response(resp).await {
            return Some(url);
        }
    }

    None
}

async fn parse_response(resp: reqwest::Response) -> Option<String> {
    if !resp.status().is_success() {
        return None;
    }
    let text = resp.text().await.ok()?;
    if text.starts_with("http") {
        return Some(text);
    }
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    if let Some(s) = json.as_str() {
        return Some(s.to_string());
    }
    let results = json.get("results").unwrap_or(&json);
    results.get("signedUrl")
        .or_else(|| results.get("signed_url"))
        .or_else(|| results.get("url"))
        .or_else(|| results.get("streamUrl"))
        .or_else(|| results.get("stream_url"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn thread_rng_nonce() -> String {
    use rand::distributions::Alphanumeric;
    use rand::{thread_rng, Rng};
    
    thread_rng()
        .sample_iter(&Alphanumeric)
        .take(32)
        .map(char::from)
        .collect()
}
