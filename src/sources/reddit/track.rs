use std::{net::IpAddr, sync::Arc};

use tracing::{debug, error};

use crate::{
    audio::{AudioFrame, processor::DecoderCommand},
    sources::{
        http::HttpTrack,
        plugin::{DecoderOutput, PlayableTrack},
    },
};

pub struct RedditTrack {
    pub client: Arc<reqwest::Client>,
    pub uri: String,
    pub audio_url: Option<String>,
    pub local_addr: Option<IpAddr>,
}

impl PlayableTrack for RedditTrack {
    fn start_decoding(&self, config: crate::config::player::PlayerConfig) -> DecoderOutput {
        let (tx, rx) = flume::bounded::<AudioFrame>((config.buffer_duration_ms / 20) as usize);
        let (cmd_tx, cmd_rx) = flume::unbounded::<DecoderCommand>();
        let (err_tx, err_rx) = flume::bounded::<String>(1);

        let stream_url = self.audio_url.clone();
        let local_addr = self.local_addr;

        let handle = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            let _guard = handle.enter();
            handle.block_on(async move {
                if let Some(url) = stream_url {
                    debug!("Reddit playback URL: {url}");
                    let http_track = HttpTrack {
                        url,
                        local_addr,
                        proxy: None,
                    };

                    let (inner_rx, inner_cmd_tx, inner_err_rx) =
                        http_track.start_decoding(config.clone());

                    // Command proxy
                    let inner_cmd_tx_clone = inner_cmd_tx.clone();
                    tokio::spawn(async move {
                        while let Ok(cmd) = cmd_rx.recv_async().await {
                            if inner_cmd_tx_clone.send(cmd).is_err() {
                                break;
                            }
                        }
                    });

                    // Error proxy
                    let err_tx_clone = err_tx.clone();
                    tokio::spawn(async move {
                        while let Ok(err) = inner_err_rx.recv_async().await {
                            let _ = err_tx_clone.send(err);
                        }
                    });

                    // Samples proxy
                    while let Ok(sample) = inner_rx.recv_async().await {
                        if tx.send(sample).is_err() {
                            break;
                        }
                    }
                } else {
                    error!("No audio stream available for Reddit track");
                    let _ = err_tx.send("Resource unavailable".to_owned());
                }
            });
        });

        (rx, cmd_tx, err_rx)
    }
}
