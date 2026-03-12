use std::{net::IpAddr, sync::Arc};

use tracing::warn;

use crate::{
    audio::{
        AudioFrame,
        processor::{AudioProcessor, DecoderCommand},
    },
    config::HttpProxyConfig,
    sources::{
        gaana::crypto::decrypt_stream_path,
        plugin::{DecoderOutput, PlayableTrack},
    },
};

pub struct GaanaTrack {
    pub client: Arc<reqwest::Client>,
    pub track_id: String,
    pub stream_quality: String,
    pub local_addr: Option<IpAddr>,
    pub proxy: Option<HttpProxyConfig>,
}

impl PlayableTrack for GaanaTrack {
    fn start_decoding(&self, config: crate::config::player::PlayerConfig) -> DecoderOutput {
        let (tx, rx) = flume::bounded::<AudioFrame>((config.buffer_duration_ms / 20) as usize);
        let (cmd_tx, cmd_rx) = flume::unbounded::<DecoderCommand>();
        let (err_tx, err_rx) = flume::bounded::<String>(1);

        let track_id = self.track_id.clone();
        let client = self.client.clone();
        let quality = self.stream_quality.clone();
        let local_addr = self.local_addr;
        let proxy = self.proxy.clone();

        tokio::spawn(async move {
            let track_id_for_log = track_id.clone();

            let hls_url = fetch_stream_url_internal(&client, &track_id, &quality).await;

            if let Some(url) = hls_url {
                let err_tx_for_setup = err_tx.clone();
                let setup_res = tokio::task::spawn_blocking(move || {
                    let is_plugin_hls = url.contains(".m3u8") || url.contains("/api/manifest/hls_");

                    let reader = if is_plugin_hls {
                        crate::sources::youtube::hls::HlsReader::new(
                            &url, local_addr, None, None, proxy,
                        )
                        .ok()
                        .map(|r| Box::new(r) as Box<dyn symphonia::core::io::MediaSource>)
                    } else {
                        super::reader::GaanaReader::new(&url, local_addr, proxy)
                            .ok()
                            .map(|r| Box::new(r) as Box<dyn symphonia::core::io::MediaSource>)
                    };

                    let kind = if is_plugin_hls {
                        Some(crate::common::types::AudioFormat::Aac)
                    } else {
                        std::path::Path::new(&url)
                            .extension()
                            .and_then(|s| s.to_str())
                            .map(crate::common::types::AudioFormat::from_ext)
                    };

                    if let Some(reader) = reader {
                        AudioProcessor::new(
                            reader,
                            kind,
                            tx,
                            cmd_rx,
                            Some(err_tx_for_setup),
                            config,
                        )
                        .map_err(|e| e.to_string())
                    } else {
                        Err("GaanaTrack: Failed to create reader".to_string())
                    }
                })
                .await
                .expect("failed to spawn gaana setup task");

                match setup_res {
                    Ok(mut processor) => {
                        std::thread::Builder::new()
                            .name(format!("gaana-decoder-{}", track_id_for_log))
                            .spawn(move || {
                                if let Err(e) = processor.run() {
                                    tracing::error!(
                                        "GaanaTrack audio processor error for {}: {}",
                                        track_id_for_log,
                                        e
                                    );
                                }
                            })
                            .expect("failed to spawn gaana decoder thread");
                    }
                    Err(e) => {
                        tracing::error!(
                            "GaanaTrack failed to initialize processor for {}: {}",
                            track_id_for_log,
                            e
                        );
                        let _ = err_tx.send(format!("Failed to initialize processor: {e}"));
                    }
                }
            } else {
                warn!("GaanaTrack: Failed to fetch stream URL for {track_id_for_log}");
            }
        });

        (rx, cmd_tx, err_rx)
    }
}

pub(super) async fn fetch_stream_url_internal(
    client: &Arc<reqwest::Client>,
    track_id: &str,
    quality: &str,
) -> Option<String> {
    let body = format!(
        "quality={}&track_id={}&stream_format=mp4",
        urlencoding::encode(quality),
        urlencoding::encode(track_id)
    );

    let resp: reqwest::Response = client
        .post("https://gaana.com/api/stream-url")
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.0.0 Safari/537.36")
        .header("Referer", "https://gaana.com/")
        .header("Origin", "https://gaana.com")
        .header("Accept", "application/json, text/plain, */*")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }

    let data: serde_json::Value = resp.json().await.ok()?;
    let encrypted_path = data.get("data")?.get("stream_path")?.as_str()?;

    decrypt_stream_path(encrypted_path)
}
