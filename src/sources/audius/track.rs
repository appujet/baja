use std::net::IpAddr;
use flume::{Receiver, Sender};
use tracing::{debug, error};

use crate::{
    audio::processor::DecoderCommand,
    sources::{http::HttpTrack, plugin::PlayableTrack},
};

pub struct AudiusTrack {
    pub client: reqwest::Client,
    pub track_id: String,
    pub stream_url: Option<String>,
    pub app_name: String,
    pub local_addr: Option<IpAddr>,
}

const API_BASE: &str = "https://discoveryprovider.audius.co";

impl PlayableTrack for AudiusTrack {
    fn start_decoding(
        &self,
    ) -> (
        Receiver<Vec<i16>>,
        Sender<DecoderCommand>,
        Receiver<String>,
    ) {
        let (tx, rx) = flume::bounded::<Vec<i16>>(64);
        let (cmd_tx, cmd_rx) = flume::unbounded::<DecoderCommand>();
        let (err_tx, err_rx) = flume::bounded::<String>(1);

        let track_id = self.track_id.clone();
        let client = self.client.clone();
        let app_name = self.app_name.clone();
        let stream_url = self.stream_url.clone();
        let local_addr = self.local_addr;

        let handle = tokio::runtime::Handle::current();
        std::thread::spawn(move || {
            let _guard = handle.enter();
            handle.block_on(async move {
                let final_url = if let Some(url) = stream_url {
                    Some(url)
                } else {
                    fetch_stream_url(&client, &track_id, &app_name).await
                };

                match final_url {
                    Some(stream_url) => {
                        debug!("Audius stream URL: {}", stream_url);
                        let http_track = HttpTrack {
                            url: stream_url,
                            local_addr,
                            proxy: None,
                        };
                        let (inner_rx, inner_cmd_tx, inner_err_rx) = http_track.start_decoding();

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
                            if let Ok(err) = inner_err_rx.recv_async().await {
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
                        error!("Failed to fetch Audius stream URL for track ID {}", track_id);
                        let _ = err_tx.send("Failed to fetch stream URL".to_string());
                    }
                }
            });
        });

        (rx, cmd_tx, err_rx)
    }
}

pub async fn fetch_stream_url(client: &reqwest::Client, track_id: &str, app_name: &str) -> Option<String> {
    let url = format!(
        "{}/v1/tracks/{}/stream?app_name={}&no_redirect=true",
        API_BASE,
        urlencoding::encode(track_id),
        urlencoding::encode(app_name)
    );

    let resp = client.get(url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }

    let body: serde_json::Value = resp.json().await.ok()?;
    body["data"].as_str().map(|s| s.to_string())
}
