use std::net::IpAddr;

use flume::{Receiver, Sender};
use regex::Regex;
use tracing::{debug, error};

use crate::{
    audio::processor::DecoderCommand,
    sources::{http::HttpTrack, plugin::PlayableTrack},
};

pub struct BandcampTrack {
    pub client: reqwest::Client,
    pub uri: String,
    pub stream_url: Option<String>,
    pub local_addr: Option<IpAddr>,
}

impl PlayableTrack for BandcampTrack {
    fn start_decoding(
        &self,
    ) -> (
        Receiver<crate::audio::buffer::PooledBuffer>,
        Sender<DecoderCommand>,
        Receiver<String>,
        Option<Receiver<std::sync::Arc<Vec<u8>>>>,
    ) {
        let (tx, rx) = flume::bounded::<crate::audio::buffer::PooledBuffer>(4);
        let (cmd_tx, cmd_rx) = flume::unbounded::<DecoderCommand>();
        let (err_tx, err_rx) = flume::bounded::<String>(1);

        let uri = self.uri.clone();
        let client = self.client.clone();
        let stream_url = self.stream_url.clone();
        let local_addr = self.local_addr;

        let handle = tokio::runtime::Handle::current();
        std::thread::spawn(move || {
            let _guard = handle.enter();
            handle.block_on(async move {
                let final_stream_url = if let Some(url) = stream_url {
                    Some(url)
                } else {
                    fetch_stream_url(&client, &uri).await
                };

                match final_stream_url {
                    Some(url) => {
                        debug!("Bandcamp stream URL: {}", url);
                        let http_track = HttpTrack {
                            url,
                            local_addr,
                            proxy: None,
                        };
                        let (inner_rx, inner_cmd_tx, inner_err_rx, _inner_opus_rx) =
                            http_track.start_decoding();

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
                        error!("Failed to fetch Bandcamp stream URL for {}", uri);
                        let _ = err_tx.send("Failed to fetch stream URL".to_string());
                    }
                }
            });
        });

        (rx, cmd_tx, err_rx, None)
    }
}

async fn fetch_stream_url(client: &reqwest::Client, uri: &str) -> Option<String> {
    let resp = client.get(uri).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }

    let body = resp.text().await.ok()?;

    let stream_re = Regex::new(r"https?://t4\.bcbits\.com/stream/[a-zA-Z0-9]+/mp3-128/\d+\?p=\d+&amp;ts=\d+&amp;t=[a-zA-Z0-9]+&amp;token=\d+_[a-zA-Z0-9]+").unwrap();

    if let Some(m) = stream_re.find(&body) {
        return Some(m.as_str().replace("&amp;", "&"));
    }

    None
}
