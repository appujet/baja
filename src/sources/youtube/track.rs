use std::{net::IpAddr, sync::Arc};

use flume::{Receiver, Sender};
use tracing::{debug, error};

use crate::{
    audio::processor::{AudioProcessor, DecoderCommand},
    configs::HttpProxyConfig,
    sources::{
        plugin::PlayableTrack,
        youtube::{
            cipher::YouTubeCipherManager, clients::YouTubeClient, hls::HlsReader,
            oauth::YouTubeOAuth,
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
    fn start_decoding(
        &self,
    ) -> (
        Receiver<crate::audio::buffer::PooledBuffer>,
        Sender<DecoderCommand>,
        flume::Receiver<String>,
    ) {
        let (tx, rx) = flume::bounded::<crate::audio::buffer::PooledBuffer>(64);
        let (cmd_tx, cmd_rx) = flume::unbounded::<DecoderCommand>();
        // unbounded so multiple consecutive errors are never silently dropped (#19)
        let (err_tx, err_rx) = flume::unbounded::<String>();

        // Prepare data for the decoding thread
        let identifier = self.identifier.clone();
        let clients = self.clients.clone();
        let oauth = self.oauth.clone();
        let cipher_manager = self.cipher_manager.clone();
        let visitor_data = self.visitor_data.clone();
        let local_addr = self.local_addr;
        let proxy = self.proxy.clone();

        // ── Phase 1: resolve URL asynchronously (no block_on) ────────────────
        // URL resolution is a network call — running it inside a tokio::spawn
        // keeps the async executor efficient. We send the result (url, client_name)
        // over a oneshot to the blocking decode task below.
        let (url_tx, url_rx) = tokio::sync::oneshot::channel::<(String, String)>();
        let identifier_async = identifier.clone();
        let err_tx_async = err_tx.clone();
        let cipher_manager_async = cipher_manager.clone();
        let oauth_async = oauth.clone();
        let clients_async = clients.clone();

        tokio::spawn(async move {
            let context = serde_json::json!({ "visitorData": visitor_data });

            for client in &clients_async {
                let client_name = client.name().to_string();
                debug!(
                    "YoutubeTrack: Resolving '{}' using {}",
                    identifier_async, client_name
                );

                match client
                    .get_track_url(
                        &identifier_async,
                        &context,
                        cipher_manager_async.clone(),
                        oauth_async.clone(),
                    )
                    .await
                {
                    Ok(Some(url)) => {
                        debug!(
                            "YoutubeTrack: Resolved stream URL via {}: {}",
                            client_name, url
                        );
                        // Best-effort send; decode task may have been dropped.
                        let _ = url_tx.send((url, client_name));
                        return;
                    }
                    Ok(None) => {
                        debug!("YoutubeTrack: {} returned no stream URL", client_name);
                    }
                    Err(e) => {
                        debug!("YoutubeTrack: {} failed to resolve: {}", client_name, e);
                    }
                }
            }

            // All clients exhausted.
            let msg = format!(
                "YoutubeTrack: All clients failed to resolve '{}'",
                identifier_async
            );
            error!("{}", msg);
            let _ = err_tx_async.send(msg);
            // url_tx is dropped here → url_rx.await will return Err, decode task exits.
        });

        // ── Phase 2: wait for URL then run CPU-bound decode in spawn_blocking ──
        // spawn_blocking gets its own OS thread from tokio's blocking pool —
        // appropriate for `AudioProcessor::run()` which loops synchronously.
        let tx_clone = tx;
        let cmd_rx_clone = cmd_rx;
        let err_tx_clone = err_tx;
        let identifier_decode = identifier;
        let cipher_manager_decode = cipher_manager;
        tokio::task::spawn_blocking(move || {
            // Block this OS thread (blocking pool) until the URL arrives.
            // This is cheaper than block_on(async { network call }) per client.
            let (url, client_name) = match url_rx.blocking_recv() {
                Ok(pair) => pair,
                Err(_) => {
                    // url_tx was dropped (all clients failed) — error already sent.
                    return;
                }
            };

            // Reader creation is synchronous/cheap (1-byte probe or HLS bootstrap).
            let reader: Box<dyn symphonia::core::io::MediaSource> =
                if url.contains(".m3u8") || url.contains("/playlist") {
                    let player_url = if url.contains("youtube.com") {
                        Some(url.clone())
                    } else {
                        None
                    };
                    match HlsReader::new(
                        &url,
                        local_addr,
                        Some(cipher_manager_decode.clone()),
                        player_url,
                        proxy.clone(),
                    ) {
                        Ok(r) => Box::new(r),
                        Err(e) => {
                            error!(
                                "YoutubeTrack: HlsReader initialization failed for {}: {}",
                                client_name, e
                            );
                            return;
                        }
                    }
                } else {
                    match super::reader::YoutubeReader::new(&url, local_addr, proxy.clone()) {
                        Ok(r) => Box::new(r),
                        Err(e) => {
                            error!(
                                "YoutubeTrack: YoutubeReader initialization failed for {}: {}",
                                client_name, e
                            );
                            return;
                        }
                    }
                };

            // Determine codec from itag param → mime= → path extension.
            let is_hls = url.contains(".m3u8") || url.contains("/playlist");
            let kind = if is_hls {
                Some(crate::common::types::AudioKind::Aac)
            } else {
                let itag: Option<u32> = url.split('?').nth(1).and_then(|qs| {
                    qs.split('&').find_map(|kv| {
                        let mut parts = kv.splitn(2, '=');
                        if parts.next() == Some("itag") {
                            parts.next().and_then(|v| v.parse().ok())
                        } else {
                            None
                        }
                    })
                });
                match itag {
                    Some(249) | Some(250) | Some(251) => {
                        Some(crate::common::types::AudioKind::Webm)
                    }
                    Some(139) | Some(140) | Some(141) => Some(crate::common::types::AudioKind::Mp4),
                    _ => {
                        if url.contains("mime=audio%2Fwebm") || url.contains("mime=audio/webm") {
                            Some(crate::common::types::AudioKind::Webm)
                        } else if url.contains("mime=audio%2Fmp4") || url.contains("mime=audio/mp4")
                        {
                            Some(crate::common::types::AudioKind::Mp4)
                        } else {
                            std::path::Path::new(url.split('?').next().unwrap_or(&url))
                                .extension()
                                .and_then(|s| s.to_str())
                                .and_then(crate::common::types::AudioKind::from_ext)
                        }
                    }
                }
            };

            match AudioProcessor::new(reader, kind, tx_clone, cmd_rx_clone, Some(err_tx_clone)) {
                Ok(mut processor) => {
                    debug!(
                        "YoutubeTrack: Playback session started for {} using {}",
                        identifier_decode, client_name
                    );
                    if let Err(e) = processor.run() {
                        error!("YoutubeTrack: Decoding session finished with error: {}", e);
                    }
                }
                Err(e) => {
                    error!(
                        "YoutubeTrack: AudioProcessor initialization failed with {}: {}",
                        client_name, e
                    );
                }
            }
        });

        (rx, cmd_tx, err_rx)
    }
}
