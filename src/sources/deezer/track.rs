use std::{net::IpAddr, sync::Arc};

use flume::{Receiver, Sender};
use tracing::{debug, error};

use crate::{
    audio::processor::{AudioProcessor, DecoderCommand},
    configs::HttpProxyConfig,
    sources::{deezer::reader::DeezerReader, plugin::PlayableTrack},
};

pub struct DeezerTrack {
    pub client: reqwest::Client,
    pub track_id: String,
    pub arl_index: usize,
    pub token_tracker: Arc<crate::sources::deezer::manager::DeezerTokenTracker>,
    pub master_key: String,
    pub local_addr: Option<IpAddr>,
    pub proxy: Option<HttpProxyConfig>,
}

impl PlayableTrack for DeezerTrack {
    fn start_decoding(&self) -> (Receiver<i16>, Sender<DecoderCommand>) {
        let (tx, rx) = flume::bounded::<i16>(4096 * 4);
        let (cmd_tx, cmd_rx) = flume::unbounded::<DecoderCommand>();

        let track_id = self.track_id.clone();
        let client = self.client.clone();
        let token_tracker = self.token_tracker.clone();
        let master_key = self.master_key.clone();
        let local_addr = self.local_addr;
        let proxy = self.proxy.clone();

        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();

            let playback_url = runtime.block_on(async {
                let mut retry_count = 0;
                let max_retries = 3;

                loop {
                    if retry_count > max_retries { break None; }

                    let tokens = match token_tracker.get_token().await {
                        Some(t) => t,
                        None => {
                            retry_count += 1;
                            continue;
                        }
                    };

                    // 1. Get Track Token
                    let url = format!(
                        "https://www.deezer.com/ajax/gw-light.php?method=song.getData&input=3&api_version=1.0&api_token={}",
                        tokens.api_token
                    );
                    let body = serde_json::json!({ "sng_id": track_id });

                    let res = match client.post(&url)
                        .header("Cookie", format!("sid={}; dzr_uniq_id={}", tokens.session_id, tokens.dzr_uniq_id))
                        .json(&body)
                        .send()
                        .await
                    {
                        Ok(r) => r,
                        Err(_) => {
                            retry_count += 1;
                            continue;
                        }
                    };

                    let json: serde_json::Value = match res.json().await {
                        Ok(v) => v,
                        Err(_) => {
                            retry_count += 1;
                            continue;
                        }
                    };

                    if let Some(error) = json.get("error").and_then(|v| v.as_array()).filter(|v| !v.is_empty()) {
                        debug!("DeezerTrack: API error: {:?}", error);
                        token_tracker.invalidate_token(tokens.arl_index).await;
                        retry_count += 1;
                        continue;
                    }

                    let track_token = match json.get("results").and_then(|r| r.get("TRACK_TOKEN")).and_then(|v| v.as_str()) {
                        Some(t) => t,
                        None => {
                            token_tracker.invalidate_token(tokens.arl_index).await;
                            retry_count += 1;
                            continue;
                        }
                    };

                    // 2. Get Media URL
                    let media_url = "https://media.deezer.com/v1/get_url";
                    let media_body = serde_json::json!({
                        "license_token": tokens.license_token,
                        "media": [{
                            "type": "FULL",
                            "formats": [
                                { "cipher": "BF_CBC_STRIPE", "format": "MP3_128" },
                                { "cipher": "BF_CBC_STRIPE", "format": "MP3_64" }
                            ]
                        }],
                        "track_tokens": [track_token]
                    });

                    let res = match client.post(media_url).json(&media_body).send().await {
                        Ok(r) => r,
                        Err(_) => {
                            retry_count += 1;
                            continue;
                        }
                    };

                    let json: serde_json::Value = match res.json().await {
                        Ok(v) => v,
                        Err(_) => {
                            retry_count += 1;
                            continue;
                        }
                    };

                    if let Some(errors) = json.get("data").and_then(|d| d.get(0)).and_then(|d| d.get("errors")).and_then(|e| e.as_array()).filter(|e| !e.is_empty()) {
                        debug!("DeezerTrack: get_url errors: {:?}", errors);
                        token_tracker.invalidate_token(tokens.arl_index).await;
                        retry_count += 1;
                        continue;
                    }

                    let url_opt = json.get("data").and_then(|d| d.get(0)).and_then(|d| d.get("media")).and_then(|m| m.get(0)).and_then(|m| m.get("sources")).and_then(|s| s.get(0)).and_then(|s| s.get("url")).and_then(|u| u.as_str());

                    if let Some(url) = url_opt {
                        return Some(format!("deezer_encrypted:{}:{}", track_id, url));
                    } else {
                        token_tracker.invalidate_token(tokens.arl_index).await;
                        retry_count += 1;
                        continue;
                    }
                }
            });

            if let Some(url) = playback_url {
                let custom_reader = if url.starts_with("deezer_encrypted:") {
                    let parts: Vec<&str> = url.splitn(3, ':').collect();
                    if parts.len() == 3 {
                        let track_id = parts[1];
                        let media_url = parts[2];
                        DeezerReader::new(
                            media_url,
                            track_id,
                            &master_key,
                            local_addr,
                            proxy.clone(),
                        )
                        .ok()
                        .map(|r| Box::new(r) as Box<dyn symphonia::core::io::MediaSource>)
                    } else {
                        None
                    }
                } else {
                    None
                };

                let reader = custom_reader.unwrap_or_else(|| {
                    Box::new(
                        crate::audio::RemoteReader::new(&url, local_addr, proxy.clone()).unwrap(),
                    ) as Box<dyn symphonia::core::io::MediaSource>
                });

                let ext_hint = std::path::Path::new(&url)
                    .extension()
                    .and_then(|s| s.to_str());

                match AudioProcessor::new(reader, ext_hint, tx, cmd_rx) {
                    Ok(mut processor) => {
                        if let Err(e) = processor.run() {
                            error!("DeezerTrack audio processor error: {}", e);
                        }
                    }
                    Err(e) => {
                        error!("DeezerTrack failed to initialize processor: {}", e);
                    }
                }
            } else {
                error!(
                    "DeezerTrack: Failed to resolve playback URL for {}",
                    track_id
                );
            }
        });

        (rx, cmd_tx)
    }
}
