use std::{net::IpAddr, sync::Arc};

use tracing::{debug, error};

use crate::{
    audio::{AudioFrame, processor::DecoderCommand},
    sources::{
        http::HttpTrack,
        plugin::{DecoderOutput, PlayableTrack},
    },
};

pub struct AudiusTrack {
    pub client: Arc<reqwest::Client>,
    pub track_id: String,
    pub stream_url: Option<String>,
    pub app_name: String,
    pub local_addr: Option<IpAddr>,
}

const API_BASE: &str = "https://discoveryprovider.audius.co";

impl PlayableTrack for AudiusTrack {
    fn start_decoding(&self, config: crate::config::player::PlayerConfig) -> DecoderOutput {
        let (tx, rx) = flume::bounded::<AudioFrame>((config.buffer_duration_ms / 20) as usize);
        let (cmd_tx, cmd_rx) = flume::unbounded::<DecoderCommand>();
        let (err_tx, err_rx) = flume::bounded::<String>(1);

        let track_id = self.track_id.clone();
        let client = self.client.clone();
        let app_name = self.app_name.clone();
        let stream_url = self.stream_url.clone();
        let local_addr = self.local_addr;

        let handle = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            let _guard = handle.enter();
            handle.block_on(async move {
                let final_url = if let Some(url) = stream_url {
                    Some(url)
                } else {
                    fetch_stream_url(&client, &track_id, &app_name).await
                };

                match final_url {
                    Some(stream_url) => {
                        debug!("Audius stream URL: {stream_url}");
                        let http_track = HttpTrack {
                            url: stream_url,
                            local_addr,
                            proxy: None,
                        };
                        let (inner_rx, inner_cmd_tx, inner_err_rx) =
                            http_track.start_decoding(config.clone());

                        // Proxy commands
                        let inner_cmd_tx_clone = inner_cmd_tx.clone();
                        tokio::spawn(async move {
                            while let Ok(cmd) = cmd_rx.recv_async().await {
                                if inner_cmd_tx_clone.send(cmd).is_err() {
                                    break;
                                }
                            }
                        });

                        // Proxy errors
                        let err_tx_clone = err_tx.clone();
                        tokio::spawn(async move {
                            while let Ok(err) = inner_err_rx.recv_async().await {
                                let _ = err_tx_clone.send(err);
                            }
                        });

                        // Proxy samples
                        while let Ok(sample) = inner_rx.recv_async().await {
                            if tx.send(sample).is_err() {
                                break;
                            }
                        }
                    }
                    None => {
                        error!("Failed to fetch Audius stream URL for track ID {track_id}");
                        let _ = err_tx.send("Failed to fetch stream URL".to_owned());
                    }
                }
            });
        });

        (rx, cmd_tx, err_rx)
    }
}

pub async fn fetch_stream_url(
    client: &Arc<reqwest::Client>,
    track_id: &str,
    app_name: &str,
) -> Option<String> {
    let url = format!(
        "{API_BASE}/v1/tracks/{}/stream",
        urlencoding::encode(track_id)
    );

    let resp = client
        .get(url)
        .query(&[("app_name", app_name), ("no_redirect", "true")])
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }

    let body: serde_json::Value = resp.json().await.ok()?;
    body["data"].as_str().map(|s| s.to_owned())
}
