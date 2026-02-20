use std::{net::IpAddr, sync::Arc};

use base64::Engine;
use flume::{Receiver, Sender};
use tracing::{debug, error};

use crate::{
    audio::processor::{AudioProcessor, DecoderCommand},
    configs::HttpProxyConfig,
    sources::{
        plugin::PlayableTrack,
        youtube::{
            cipher::YouTubeCipherManager,
            clients::YouTubeClient,
            hls::HlsReader,
            oauth::YouTubeOAuth,
            sabr::{reader::SabrReader, structs::FormatId},
        },
    },
};

pub struct YoutubeTrack {
    pub identifier: String,
    pub clients: Vec<Arc<dyn YouTubeClient>>,
    pub oauth: Arc<YouTubeOAuth>,
    pub cipher_manager: Arc<YouTubeCipherManager>,
    pub visitor_data: Option<String>,
    pub local_addr: Option<IpAddr>,
    pub proxy: Option<HttpProxyConfig>,
}

impl PlayableTrack for YoutubeTrack {
    fn start_decoding(&self) -> (Receiver<i16>, Sender<DecoderCommand>) {
        let (tx, rx) = flume::bounded::<i16>(4096 * 4);
        let (cmd_tx, cmd_rx) = flume::unbounded::<DecoderCommand>();

        let identifier = self.identifier.clone();
        let clients = self.clients.clone();
        let oauth = self.oauth.clone();
        let cipher_manager = self.cipher_manager.clone();
        let visitor_data = self.visitor_data.clone();
        let local_addr = self.local_addr;
        let proxy = self.proxy.clone();

        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();

            let context = if let Some(vd) = visitor_data {
                serde_json::json!({ "visitorData": vd })
            } else {
                serde_json::json!({})
            };

            let playback_url = runtime.block_on(async {
                for client in &clients {
                    debug!(
                        "YoutubeTrack: Resolving playback URL for '{}' using {}",
                        identifier,
                        client.name()
                    );
                    match client
                        .get_track_url(&identifier, &context, cipher_manager.clone(), oauth.clone())
                        .await
                    {
                        Ok(Some(url)) => return Some(url),
                        Ok(None) => continue,
                        Err(e) => {
                            error!(
                                "YoutubeTrack: Playback URL error with {}: {}",
                                client.name(),
                                e
                            );
                        }
                    }
                }
                None
            });

            if let Some(url) = playback_url {
                let custom_reader = if url.starts_with("sabr://") {
                    let data_str = url.strip_prefix("sabr://").unwrap_or("");
                    if let Ok(data_bytes) =
                        base64::engine::general_purpose::STANDARD.decode(data_str.as_bytes())
                    {
                        if let Ok(data) = serde_json::from_slice::<serde_json::Value>(&data_bytes) {
                            let streaming_url = data
                                .get("url")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let config_str =
                                data.get("config").and_then(|v| v.as_str()).unwrap_or("");
                            if let Ok(config) = base64::engine::general_purpose::STANDARD
                                .decode(config_str.as_bytes())
                            {
                                let client_name =
                                    data.get("clientName").and_then(|v| v.as_i64()).unwrap_or(0)
                                        as i32;
                                let client_version = data
                                    .get("clientVersion")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let visitor_data = data
                                    .get("visitorData")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let formats: Vec<FormatId> = data
                                    .get("formats")
                                    .and_then(|v| v.as_array())
                                    .map(|arr| {
                                        arr.iter()
                                            .filter_map(|f| serde_json::from_value(f.clone()).ok())
                                            .collect()
                                    })
                                    .unwrap_or_default();

                                let (reader, _) = SabrReader::new(
                                    streaming_url,
                                    config,
                                    client_name,
                                    client_version,
                                    visitor_data,
                                    formats,
                                );
                                Some(Box::new(reader) as Box<dyn symphonia::core::io::MediaSource>)
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else if url.contains(".m3u8") || url.contains("/playlist") {
                    let player_url = if url.contains("youtube.com") {
                        Some(url.clone())
                    } else {
                        None
                    };
                    HlsReader::new(
                        &url,
                        local_addr,
                        Some(cipher_manager.clone()),
                        player_url,
                        proxy.clone(),
                    )
                    .ok()
                    .map(|r| Box::new(r) as Box<dyn symphonia::core::io::MediaSource>)
                } else {
                    None
                };

                let reader = custom_reader.unwrap_or_else(|| {
                    Box::new(
                        crate::audio::RemoteReader::new(&url, local_addr, proxy.clone()).unwrap(),
                    ) as Box<dyn symphonia::core::io::MediaSource>
                });

                let ext_hint = if url.contains(".m3u8") || url.contains("/api/manifest/hls_") {
                    Some("aac")
                } else {
                    std::path::Path::new(&url)
                        .extension()
                        .and_then(|s| s.to_str())
                };

                match AudioProcessor::new(reader, ext_hint, tx, cmd_rx) {
                    Ok(mut processor) => {
                        if let Err(e) = processor.run() {
                            error!("YoutubeTrack: Audio processor error: {}", e);
                        }
                    }
                    Err(e) => {
                        error!("YoutubeTrack: Failed to initialize audio processor: {}", e);
                    }
                }
            } else {
                error!(
                    "YoutubeTrack: Failed to resolve playback URL for {}",
                    identifier
                );
            }
        });

        (rx, cmd_tx)
    }
}
