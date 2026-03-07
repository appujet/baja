use std::sync::Arc;

use base64::{Engine as _, engine::general_purpose};
use tracing::{debug, error, info};

use super::{
    client::TidalClient,
    model::{Manifest, PlaybackInfo},
};
use crate::{
    audio::{
        processor::{AudioProcessor, DecoderCommand},
        source::HttpSource,
    },
    common::types::AudioFormat,
    sources::plugin::{DecoderOutput, PlayableTrack},
};

pub struct TidalTrack {
    pub identifier: String,
    pub client: Arc<TidalClient>,
}

impl PlayableTrack for TidalTrack {
    fn start_decoding(&self, config: crate::config::player::PlayerConfig) -> DecoderOutput {
        let (tx, rx) = flume::bounded((config.buffer_duration_ms / 20) as usize);
        let (cmd_tx, cmd_rx) = flume::bounded(8);
        let (err_tx, err_rx) = flume::bounded(1);

        let identifier = self.identifier.clone();
        let tidal = self.client.clone();

        tokio::spawn(async move {
            let quality = &tidal.quality;
            let path = format!(
                "/tracks/{}/playbackinfo?audioquality={}&playbackmode=STREAM&assetpresentation=FULL",
                identifier, quality
            );

            debug!("TidalTrack: Fetching playback info for {}", identifier);

            let token = match tidal.token_tracker.get_oauth_token().await {
                Some(t) => t,
                None => {
                    let _ = err_tx.send("Tidal playback requires an OAuth login".to_string());
                    return;
                }
            };

            let url = format!(
                "https://api.tidal.com/v1{path}&countryCode={}",
                tidal.country_code
            );
            let resp = match tidal
                .inner
                .get(&url)
                .header("Authorization", format!("Bearer {token}"))
                .header("User-Agent", "TIDAL/3704 CFNetwork/1220.1 Darwin/20.3.0")
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    error!("TidalTrack: Failed to fetch playback info: {}", e);
                    let _ = err_tx.send(format!("Failed to fetch playback info: {}", e));
                    return;
                }
            };

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                error!("TidalTrack: API returned {}: {}", status, body);
                let _ = err_tx.send(format!("Tidal API returned error: {}", status));
                return;
            }

            let info: PlaybackInfo = match resp.json().await {
                Ok(i) => i,
                Err(e) => {
                    error!("TidalTrack: Failed to parse playback info: {}", e);
                    let _ = err_tx.send(format!("Failed to parse playback info: {}", e));
                    return;
                }
            };

            let decoded = match general_purpose::STANDARD.decode(&info.manifest) {
                Ok(d) => d,
                Err(e) => {
                    error!("TidalTrack: Failed to decode manifest: {}", e);
                    let _ = err_tx.send(format!("Failed to decode manifest: {}", e));
                    return;
                }
            };

            let manifest: Manifest = match serde_json::from_slice(&decoded) {
                Ok(m) => m,
                Err(e) => {
                    error!("TidalTrack: Failed to parse manifest JSON: {}", e);
                    let _ = err_tx.send(format!("Failed to parse manifest: {}", e));
                    return;
                }
            };

            let stream_url = match manifest.urls.first() {
                Some(u) => u.clone(),
                None => {
                    error!("TidalTrack: No stream URL in manifest");
                    let _ = err_tx.send("No stream URL in manifest".to_string());
                    return;
                }
            };

            info!(
                "TidalTrack: Starting playback for {} with quality {}",
                identifier, quality
            );

            let mut kind = AudioFormat::from_url(&stream_url);
            if kind == AudioFormat::Unknown {
                kind = if quality == "LOSSLESS" || quality == "HI_RES_LOSSLESS" {
                    AudioFormat::Flac
                } else {
                    AudioFormat::Aac
                };
            }

            let stream_url_clone = stream_url.clone();
            let client_clone = (*tidal.inner).clone();
            let reader_res = tokio::task::spawn_blocking(move || {
                HttpSource::new(client_clone, &stream_url_clone)
            })
            .await
            .expect("TidalTrack: reader spawn_blocking failed");

            let reader = match reader_res {
                Ok(r) => r,
                Err(e) => {
                    error!("TidalTrack: Failed to initialize HttpSource: {}", e);
                    let _ = err_tx.send(format!("Failed to initialize source: {}", e));
                    return;
                }
            };

            let (inner_cmd_tx, inner_cmd_rx) = flume::bounded(8);
            let tx_clone = tx.clone();
            let err_tx_clone = err_tx.clone();

            let mut process_task = tokio::task::spawn_blocking(move || {
                match AudioProcessor::new(
                    Box::new(reader),
                    Some(kind),
                    tx_clone,
                    inner_cmd_rx,
                    Some(err_tx_clone),
                    config,
                ) {
                    Ok(mut p) => p.run().map_err(|e| e.to_string()),
                    Err(e) => {
                        error!("TidalTrack: AudioProcessor initialization failed: {}", e);
                        Err(format!("Processor init failed: {}", e))
                    }
                }
            });

            loop {
                tokio::select! {
                    cmd = cmd_rx.recv_async() => {
                        match cmd {
                            Ok(DecoderCommand::Seek(ms)) => {
                                let _ = inner_cmd_tx.send(DecoderCommand::Seek(ms));
                            }
                            Ok(DecoderCommand::Stop) | Err(_) => {
                                let _ = inner_cmd_tx.send(DecoderCommand::Stop);
                                return;
                            }
                        }
                    }
                    res = &mut process_task => {
                        if let Err(e) = res {
                            error!("TidalTrack: Join error: {}", e);
                        }
                        return;
                    }
                }
            }
        });

        (rx, cmd_tx, err_rx, None)
    }
}
