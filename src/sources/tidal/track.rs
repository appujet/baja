use std::sync::Arc;

use tracing::{error, info};

use super::client::TidalClient;
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
    pub stream_url: String,
    pub kind: AudioFormat,
    pub client: Arc<TidalClient>,
}

impl PlayableTrack for TidalTrack {
    fn start_decoding(&self, config: crate::config::player::PlayerConfig) -> DecoderOutput {
        let (tx, rx) = flume::bounded((config.buffer_duration_ms / 20) as usize);
        let (cmd_tx, cmd_rx) = flume::bounded(8);
        let (err_tx, err_rx) = flume::bounded(1);

        let identifier = self.identifier.clone();
        let tidal = self.client.clone();
        let stream_url = self.stream_url.clone();
        let kind = self.kind;

        tokio::spawn(async move {
            info!(
                "TidalTrack: Starting playback for {} with quality {}",
                identifier, tidal.quality
            );

            let client_clone = (*tidal.inner).clone();
            let reader_res = tokio::task::spawn_blocking(move || {
                HttpSource::new(client_clone, &stream_url)
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
