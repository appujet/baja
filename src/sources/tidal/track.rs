// Copyright (c) 2026 appujet, notdeltaxd and contributors
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::sync::Arc;

use tracing::{debug, error};

use super::client::TidalClient;
use crate::{
    audio::{AudioFrame, processor::AudioProcessor, source::HttpSource},
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
        let (tx, rx) = flume::bounded::<AudioFrame>((config.buffer_duration_ms / 20) as usize);
        let (cmd_tx, cmd_rx) = flume::bounded(8);
        let (err_tx, err_rx) = flume::bounded(1);

        let identifier = self.identifier.clone();
        let tidal = self.client.clone();
        let stream_url = self.stream_url.clone();
        let kind = self.kind;

        let err_tx_for_setup = err_tx.clone();
        tokio::spawn(async move {
            debug!(
                "TidalTrack: Starting playback for {} with quality {}",
                identifier, tidal.quality
            );

            let setup_res = tokio::task::spawn_blocking(move || {
                let client_clone = (*tidal.inner).clone();
                match HttpSource::new(client_clone, &stream_url) {
                    Ok(reader) => AudioProcessor::new(
                        Box::new(reader),
                        Some(kind),
                        tx,
                        cmd_rx,
                        Some(err_tx_for_setup),
                        config,
                    )
                    .map_err(|e| e.to_string()),
                    Err(e) => {
                        error!("TidalTrack: Failed to initialize HttpSource: {}", e);
                        Err(format!("Failed to initialize source: {}", e))
                    }
                }
            })
            .await
            .expect("TidalTrack: reader setup spawn_blocking failed");

            match setup_res {
                Ok(mut processor) => {
                    std::thread::Builder::new()
                        .name(format!("tidal-decoder-{}", identifier))
                        .spawn(move || {
                            if let Err(e) = processor.run() {
                                error!(
                                    "TidalTrack audio processor error for {}: {}",
                                    identifier, e
                                );
                            }
                        })
                        .expect("failed to spawn tidal decoder thread");
                }
                Err(e) => {
                    error!(
                        "TidalTrack failed to initialize processor for {}: {}",
                        identifier, e
                    );
                    let _ = err_tx.send(format!("Failed to initialize processor: {e}"));
                }
            }
        });

        (rx, cmd_tx, err_rx)
    }
}
