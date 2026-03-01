use std::{net::IpAddr, sync::Arc};

use flume::{Receiver, Sender};
use regex::Regex;
use tracing::{debug, error};

use super::utils;
use crate::{
    audio::processor::DecoderCommand,
    sources::{http::HttpTrack, plugin::PlayableTrack},
};

pub struct YandexMusicTrack {
    pub client: Arc<reqwest::Client>,
    pub track_id: String,
    pub local_addr: Option<IpAddr>,
    pub proxy: Option<crate::configs::HttpProxyConfig>,
}

impl PlayableTrack for YandexMusicTrack {
    fn start_decoding(
        &self,
        config: crate::configs::player::PlayerConfig,
    ) -> (
        Receiver<crate::audio::buffer::PooledBuffer>,
        Sender<DecoderCommand>,
        Receiver<String>,
        Option<Receiver<std::sync::Arc<Vec<u8>>>>,
    ) {
        let (tx, rx) = flume::bounded::<crate::audio::buffer::PooledBuffer>(
            (config.buffer_duration_ms / 20) as usize,
        );
        let (cmd_tx, cmd_rx) = flume::unbounded::<DecoderCommand>();
        let (err_tx, err_rx) = flume::bounded::<String>(1);

        let track_id = self.track_id.clone();
        let client = self.client.clone();
        let local_addr = self.local_addr;
        let proxy = self.proxy.clone();

        let handle = tokio::runtime::Handle::current();
        std::thread::spawn(move || {
            let _guard = handle.enter();
            handle.block_on(async move {
                match fetch_download_url(&client, &track_id).await {
                    Some(stream_url) => {
                        debug!("Yandex Music stream URL: {}", stream_url);
                        let http_track = HttpTrack {
                            url: stream_url,
                            local_addr,
                            proxy,
                        };
                        let (inner_rx, inner_cmd_tx, inner_err_rx, _inner_opus_rx) =
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
                        error!(
                            "Failed to fetch Yandex Music stream URL for track ID {}",
                            track_id
                        );
                        let _ = err_tx.send("Failed to fetch stream URL".to_string());
                    }
                }
            });
        });

        (rx, cmd_tx, err_rx, None)
    }
}

pub(crate) async fn fetch_download_url(client: &Arc<reqwest::Client>, id: &str) -> Option<String> {
    let url = format!("https://api.music.yandex.net/tracks/{}/download-info", id);
    let resp = client.get(url).send().await.ok()?;
    let data: serde_json::Value = resp.json().await.ok()?;

    let results = data["result"].as_array()?;

    // Find MP3 with highest bitrate
    let mut mp3_items: Vec<_> = results
        .iter()
        .filter(|item| item["codec"].as_str() == Some("mp3"))
        .collect();

    mp3_items.sort_by_key(|item| item["bitrateInKbps"].as_u64().unwrap_or(0));
    let best_mp3 = mp3_items.last()?;
    let download_info_url = best_mp3["downloadInfoUrl"].as_str()?;

    // Fetch XML
    let xml_resp = client.get(download_info_url).send().await.ok()?;
    let xml_text = xml_resp.text().await.ok()?;

    let get_tag = |text: &str, tag: &str| -> Option<String> {
        let pattern = format!("<{tag}>(?P<val>[^<]+)</{tag}>");
        let re = Regex::new(&pattern).ok()?;
        re.captures(text)?.name("val")?.as_str().to_string().into()
    };

    let host: String = get_tag(&xml_text, "host")?;
    let path: String = get_tag(&xml_text, "path")?;
    let ts: String = get_tag(&xml_text, "ts")?;
    let s: String = get_tag(&xml_text, "s")?;

    let md5 = utils::generate_download_sign(&path, &s);

    Some(format!("https://{}/get-mp3/{}/{}{}", host, md5, ts, path))
}
