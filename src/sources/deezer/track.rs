use std::{net::IpAddr, sync::Arc};

use tracing::{debug, error};

use crate::{
    audio::{
        AudioFrame,
        processor::{AudioProcessor, DecoderCommand},
    },
    config::HttpProxyConfig,
    sources::{
        deezer::reader::DeezerReader,
        plugin::{DecoderOutput, PlayableTrack},
    },
};

pub struct DeezerTrack {
    pub client: Arc<reqwest::Client>,
    pub track_id: String,
    pub token_tracker: Arc<crate::sources::deezer::token::DeezerTokenTracker>,
    pub master_key: String,
    pub local_addr: Option<IpAddr>,
    pub proxy: Option<HttpProxyConfig>,
}

impl PlayableTrack for DeezerTrack {
    fn start_decoding(&self, config: crate::config::player::PlayerConfig) -> DecoderOutput {
        let (tx, rx) = flume::bounded::<AudioFrame>((config.buffer_duration_ms / 20) as usize);
        let (cmd_tx, cmd_rx) = flume::unbounded::<DecoderCommand>();
        let (err_tx, err_rx) = flume::bounded::<String>(1);

        let track_id = self.track_id.clone();
        let client = self.client.clone();
        let token_tracker = self.token_tracker.clone();
        let master_key = self.master_key.clone();
        let local_addr = self.local_addr;
        let proxy = self.proxy.clone();

        tokio::spawn(async move {
            let track_id_for_log = track_id.clone();

            let playback_url = async {
                let mut retry_count = 0;
                let max_retries = 3;

                loop {
                    if retry_count > max_retries {
                        break None;
                    }

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

                    let res = match client
                        .post(&url)
                        .header(
                            "Cookie",
                            format!(
                                "sid={}; dzr_uniq_id={}",
                                tokens.session_id, tokens.dzr_uniq_id
                            ),
                        )
                        .json(&body)
                        .send()
                        .await
                    {
                        Ok(r) => r,
                        Err(e) => {
                            debug!("DeezerTrack: Failed to get song data: {e}");
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

                    if let Some(error) = json
                        .get("error")
                        .and_then(|v| v.as_array())
                        .filter(|v| !v.is_empty())
                    {
                        debug!("DeezerTrack: API error: {error:?}");
                        token_tracker.invalidate_token(tokens.arl_index).await;
                        retry_count += 1;
                        continue;
                    }

                    let track_token = match json
                        .get("results")
                        .and_then(|r| r.get("TRACK_TOKEN"))
                        .and_then(|v| v.as_str())
                    {
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
                        Err(e) => {
                            debug!("DeezerTrack: Failed to get media URL: {e}");
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

                    if let Some(errors) = json
                        .get("data")
                        .and_then(|d| d.get(0))
                        .and_then(|d| d.get("errors"))
                        .and_then(|e| e.as_array())
                        .filter(|e| !e.is_empty())
                    {
                        debug!("DeezerTrack: get_url errors: {errors:?}");
                        token_tracker.invalidate_token(tokens.arl_index).await;
                        retry_count += 1;
                        continue;
                    }

                    let url_opt = json
                        .get("data")
                        .and_then(|d| d.get(0))
                        .and_then(|d| d.get("media"))
                        .and_then(|m| m.get(0))
                        .and_then(|m| m.get("sources"))
                        .and_then(|s| s.get(0))
                        .and_then(|s| s.get("url"))
                        .and_then(|u| u.as_str());

                    if let Some(url) = url_opt {
                        break Some(format!("deezer_encrypted:{track_id}:{url}"));
                    } else {
                        token_tracker.invalidate_token(tokens.arl_index).await;
                        retry_count += 1;
                        continue;
                    }
                }
            }.await;

            if let Some(url) = playback_url {
                let err_tx_for_setup = err_tx.clone();
                let setup_res = tokio::task::spawn_blocking(move || {
                    let (reader_res, final_url) = if let Some(stripped) =
                        url.strip_prefix("deezer_encrypted:")
                    {
                        let parts: Vec<&str> = stripped.splitn(2, ':').collect();
                        if parts.len() == 2 {
                            let track_id = parts[0];
                            let media_url = parts[1];
                            (
                                DeezerReader::new(
                                    media_url,
                                    track_id,
                                    &master_key,
                                    local_addr,
                                    proxy.clone(),
                                )
                                .map(|r| Box::new(r) as Box<dyn symphonia::core::io::MediaSource>),
                                media_url.to_string(),
                            )
                        } else {
                            (
                                super::reader::remote_reader::DeezerRemoteReader::new(
                                    &url,
                                    local_addr,
                                    proxy.clone(),
                                )
                                .map(|r| Box::new(r) as Box<dyn symphonia::core::io::MediaSource>),
                                url.clone(),
                            )
                        }
                    } else {
                        (
                            super::reader::remote_reader::DeezerRemoteReader::new(
                                &url,
                                local_addr,
                                proxy.clone(),
                            )
                            .map(|r| Box::new(r) as Box<dyn symphonia::core::io::MediaSource>),
                            url.clone(),
                        )
                    };

                    let reader = match reader_res {
                        Ok(r) => r,
                        Err(e) => {
                            return Err(symphonia::core::errors::Error::IoError(
                                std::io::Error::other(format!("Failed to create reader: {e}")),
                            ));
                        }
                    };

                    let url_for_extension = final_url;

                    let kind = crate::common::types::AudioFormat::from_url(&url_for_extension);

                    AudioProcessor::new(
                        reader,
                        Some(kind),
                        tx,
                        cmd_rx,
                        Some(err_tx_for_setup),
                        config,
                    )
                })
                .await
                .expect("failed to spawn deezer setup task");

                let processor = match setup_res {
                    Ok(r) => r,
                    Err(e) => {
                        error!(
                            "DeezerTrack failed to initialize processor for {}: {}",
                            track_id_for_log, e
                        );
                        let _ = err_tx.send(format!("Failed to initialize processor: {e}"));
                        return;
                    }
                };

                let mut processor = processor;
                let track_id_for_thread = track_id_for_log.clone();
                let spawn_res = std::thread::Builder::new()
                    .name(format!("deezer-decoder-{}", track_id_for_log))
                    .spawn(move || {
                        if let Err(e) = processor.run() {
                            error!(
                                "DeezerTrack audio processor error for {}: {}",
                                track_id_for_thread, e
                            );
                        }
                    });

                if let Err(e) = spawn_res {
                    error!(
                        "DeezerTrack failed to spawn decoder thread for {}: {}",
                        track_id_for_log, e
                    );
                    let _ = err_tx.send(format!("Failed to spawn decoder thread: {e}"));
                }
            } else {
                error!("DeezerTrack: Failed to resolve playback URL for {track_id_for_log}");
            }
        });

        (rx, cmd_tx, err_rx)
    }
}

pub(super) async fn verify_track_resolvable(
    client: &Arc<reqwest::Client>,
    track_id: &str,
    token_tracker: &crate::sources::deezer::token::DeezerTokenTracker,
) -> Option<String> {
    let tokens = token_tracker.get_token().await?;

    let song_url = format!(
        "https://www.deezer.com/ajax/gw-light.php?method=song.getData&input=3&api_version=1.0&api_token={}",
        tokens.api_token
    );
    let json: serde_json::Value = client
        .post(&song_url)
        .header(
            "Cookie",
            format!(
                "sid={}; dzr_uniq_id={}",
                tokens.session_id, tokens.dzr_uniq_id
            ),
        )
        .json(&serde_json::json!({ "sng_id": track_id }))
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;

    if json
        .get("error")
        .and_then(|v| v.as_array())
        .is_some_and(|e| !e.is_empty())
    {
        token_tracker.invalidate_token(tokens.arl_index).await;
        return None;
    }

    let track_token = json
        .get("results")
        .and_then(|r| r.get("TRACK_TOKEN"))
        .and_then(|v| v.as_str())?
        .to_owned();

    let media_body = serde_json::json!({
        "license_token": tokens.license_token,
        "media": [{
            "type": "FULL",
            "formats": [
                { "cipher": "BF_CBC_STRIPE", "format": "MP3_128" }
            ]
        }],
        "track_tokens": [track_token]
    });

    let media_json: serde_json::Value = client
        .post("https://media.deezer.com/v1/get_url")
        .json(&media_body)
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;

    if media_json
        .get("data")
        .and_then(|d| d.get(0))
        .and_then(|d| d.get("errors"))
        .and_then(|e| e.as_array())
        .is_some_and(|e| !e.is_empty())
    {
        token_tracker.invalidate_token(tokens.arl_index).await;
        return None;
    }

    media_json
        .get("data")
        .and_then(|d| d.get(0))
        .and_then(|d| d.get("media"))
        .and_then(|m| m.get(0))
        .and_then(|m| m.get("sources"))
        .and_then(|s| s.get(0))
        .and_then(|s| s.get("url"))
        .and_then(|u| u.as_str())
        .map(|s| s.to_owned())
}
