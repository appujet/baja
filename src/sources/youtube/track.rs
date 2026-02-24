use std::{net::IpAddr, sync::Arc};

use flume::{Receiver, Sender};
use tracing::{debug, error, info, warn};

use crate::{
    audio::processor::DecoderCommand,
    configs::HttpProxyConfig,
    sources::{
        plugin::PlayableTrack,
        youtube::{
            cipher::YouTubeCipherManager, clients::YouTubeClient, oauth::YouTubeOAuth,
            sabr::SabrConfig,
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

enum ResolvedStream {
    /// Direct HTTP/HLS URL + source client name
    Url(String, String),
    /// SABR config + source client name + the client object (for re-resolution)
    Sabr(SabrConfig, String, Arc<dyn YouTubeClient>),
}

impl PlayableTrack for YoutubeTrack {
    fn start_decoding(
        &self,
    ) -> (
        Receiver<crate::audio::PooledBuffer>,
        Sender<DecoderCommand>,
        flume::Receiver<String>,
        Option<Receiver<std::sync::Arc<Vec<u8>>>>,
    ) {
        let (tx, rx) = flume::bounded(32);
        let (cmd_tx, cmd_rx) = flume::bounded(8);
        let (err_tx, err_rx) = flume::bounded(1);
        // Passthrough channel for raw Opus frames (YouTube WebM/Opus — zero transcode)
        let (opus_tx, opus_rx) = flume::bounded::<std::sync::Arc<Vec<u8>>>(32);

        let identifier_async = self.identifier.clone();
        let cipher_manager_async = self.cipher_manager.clone();
        let oauth_async = self.oauth.clone();
        let clients_async = self.clients.clone();
        let visitor_data_for_task = self.visitor_data.clone();
        let proxy_bg = self.proxy.clone();
        let local_addr_bg = self.local_addr;

        tokio::spawn(async move {
            let context = serde_json::json!({ "visitorData": visitor_data_for_task });
            let visitor_data_str = context
                .get("visitorData")
                .and_then(|v| v.as_str())
                .map(str::to_string);

            let mut current_seek_ms = 0u64;

            'playback_loop: loop {
                // 1. Resolve URL (Async resolution)
                let signature_timestamp = cipher_manager_async.get_signature_timestamp().await.ok();

                let mut resolved = None;

                for client in &clients_async {
                    let client_name = client.name().to_string();

                    // 1. Try SABR (ONLY if client is WEB)
                    if client_name == "Web" {
                        if let Some(sabr_cfg) = client
                            .get_sabr_config(
                                &identifier_async,
                                visitor_data_str.as_deref(),
                                signature_timestamp,
                                cipher_manager_async.clone(),
                                current_seek_ms,
                            )
                            .await
                        {
                            info!(
                                "YoutubeTrack: starting SABR stream for '{}' using client '{}' at {}ms",
                                identifier_async, client_name, current_seek_ms
                            );
                            resolved =
                                Some(ResolvedStream::Sabr(sabr_cfg, client_name, client.clone()));
                            break;
                        }
                    }

                    // 2. Try URL resolution (fallback)
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
                            info!(
                                "YoutubeTrack: resolved track URL for '{}' using client '{}'",
                                identifier_async, client_name
                            );
                            resolved = Some(ResolvedStream::Url(url, client_name));
                            break;
                        }
                        Ok(None) => {
                            debug!(
                                "YoutubeTrack: client {} returned no URL for {}",
                                client_name, identifier_async
                            );
                        }
                        Err(e) => {
                            warn!(
                                "YoutubeTrack: client {} failed to resolve {}: {}",
                                client_name, identifier_async, e
                            );
                        }
                    }
                }

                let resolved = match resolved {
                    Some(r) => r,
                    None => {
                        let msg = format!(
                            "YoutubeTrack: All clients failed to resolve '{}'",
                            identifier_async
                        );
                        error!("{}", msg);
                        let _ = err_tx.send(msg);
                        return; // Orchestrator exits
                    }
                };

                let is_sabr = matches!(resolved, ResolvedStream::Sabr(..));

                // 2. Build MediaSource and spawn AudioProcessor
                let (reader, kind, _client_name, opt_sabr_cmd_tx, opt_sabr_event_rx) =
                    match resolved {
                        ResolvedStream::Sabr(cfg, client_name, _client) => {
                            let mime = crate::sources::youtube::sabr::best_format_mime(&cfg);
                            let Some((rx, event_rx, cmd_tx, handle)) =
                                crate::sources::youtube::sabr::stream::start_sabr_stream(
                                    identifier_async.clone(),
                                    cfg,
                                )
                            else {
                                let msg = format!(
                                    "YoutubeTrack: SABR start_sabr_stream returned None for {}",
                                    identifier_async
                                );
                                error!("{}", msg);
                                let _ = err_tx.send(msg);
                                return;
                            };

                            let kind = mime.as_deref().and_then(|m| {
                                if m.contains("webm") {
                                    Some(crate::common::types::AudioKind::Webm)
                                } else if m.contains("mp4") {
                                    Some(crate::common::types::AudioKind::Mp4)
                                } else {
                                    None
                                }
                            });

                            (
                                Box::new(crate::sources::youtube::sabr::reader::SabrReader::new(
                                    rx, handle,
                                ))
                                    as Box<dyn symphonia::core::io::MediaSource>,
                                kind,
                                client_name,
                                Some(cmd_tx),
                                Some(event_rx),
                            )
                        }

                        ResolvedStream::Url(url, client_name) => {
                            let is_hls = url.contains(".m3u8") || url.contains("/playlist");
                            let player_url_clone = if url.contains("youtube.com") {
                                Some(url.clone())
                            } else {
                                None
                            };
                            let url_clone = url.clone();
                            let cipher_clone = cipher_manager_async.clone();
                            let proxy_clone = proxy_bg.clone();
                            let client_name_inner = client_name.clone();

                            let reader_res = tokio::task::spawn_blocking(move || {
                            if is_hls {
                                match crate::sources::youtube::hls::HlsReader::new(
                                    &url_clone,
                                    local_addr_bg,
                                    Some(cipher_clone),
                                    player_url_clone,
                                    proxy_clone,
                                ) {
                                    Ok(r) => {
                                        Ok(Box::new(r) as Box<dyn symphonia::core::io::MediaSource>)
                                    }
                                    Err(e) => Err(e),
                                }
                            } else if client_name_inner == "TV" {
                                match crate::sources::youtube::reader::YoutubeReader::new(
                                    &url_clone,
                                    local_addr_bg,
                                    proxy_clone,
                                ) {
                                    Ok(r) => {
                                        Ok(Box::new(r) as Box<dyn symphonia::core::io::MediaSource>)
                                    }
                                    Err(e) => Err(e),
                                }
                            } else {
                                match crate::audio::BaseRemoteReader::new(
                                    crate::audio::create_client(
                                        "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/119.0.0.0 Safari/537.36".to_string(),
                                        local_addr_bg,
                                        proxy_clone,
                                        None
                                    ).unwrap(),
                                    &url_clone,
                                ) {
                                    Ok(r) => {
                                        Ok(Box::new(r) as Box<dyn symphonia::core::io::MediaSource>)
                                    }
                                    Err(e) => Err(e),
                                }
                            }
                        })
                        .await
                        .expect("YoutubeTrack: reader spawn_blocking failed");

                            let reader = match reader_res {
                                Ok(r) => r,
                                Err(e) => {
                                    error!("YoutubeTrack: Reader initialization failed: {}", e);
                                    let _ = err_tx.send(e.to_string());
                                    return;
                                }
                            };

                            // Determine codec
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
                                    Some(139) | Some(140) | Some(141) => {
                                        Some(crate::common::types::AudioKind::Mp4)
                                    }
                                    _ => {
                                        if url.contains("mime=audio%2Fwebm")
                                            || url.contains("mime=audio/webm")
                                        {
                                            Some(crate::common::types::AudioKind::Webm)
                                        } else if url.contains("mime=audio%2Fmp4")
                                            || url.contains("mime=audio/mp4")
                                        {
                                            Some(crate::common::types::AudioKind::Mp4)
                                        } else {
                                            std::path::Path::new(
                                                url.split('?').next().unwrap_or(&url),
                                            )
                                            .extension()
                                            .and_then(|s| s.to_str())
                                            .and_then(crate::common::types::AudioKind::from_ext)
                                        }
                                    }
                                }
                            };

                            (reader, kind, client_name, None, None)
                        }
                    };

                // Spawn AudioProcessor on blocking thread
                let (inner_cmd_tx, inner_cmd_rx) = flume::bounded(8);
                let tx_clone = tx.clone();
                let err_tx_clone = err_tx.clone();
                let opus_tx_clone = opus_tx.clone();

                let mut process_task = tokio::task::spawn_blocking(move || {
                    match crate::audio::processor::AudioProcessor::new_with_passthrough(
                        reader,
                        kind,
                        tx_clone,
                        Some(opus_tx_clone),
                        inner_cmd_rx,
                        Some(err_tx_clone),
                    ) {
                        Ok(mut processor) => {
                            if let Err(e) = processor.run() {
                                error!("YoutubeTrack: Decoding session finished with error: {}", e);
                            }
                        }
                        Err(e) => {
                            error!("YoutubeTrack: AudioProcessor initialization failed: {}", e);
                        }
                    }
                });

                // Spawn SABR recovery orchestrator if needed
                let sabr_cmd_tx_clone = opt_sabr_cmd_tx.clone();
                let sabr_abort_tx = if let (Some(sabr_event_rx), Some(sabr_cmd_tx)) =
                    (opt_sabr_event_rx, sabr_cmd_tx_clone)
                {
                    let id_rec = identifier_async.clone();
                    let visitor_data_rec = visitor_data_str.clone();
                    let cipher_rec = cipher_manager_async.clone();
                    let clients_rec = clients_async.clone();

                    let (abort_tx, abort_rx) = tokio::sync::oneshot::channel();

                    tokio::spawn(async move {
                        tokio::select! {
                            _ = abort_rx => {
                                // aborted by seek or stop
                            }
                            event_opt = sabr_event_rx.recv_async() => {
                                if let Ok(event) = event_opt {
                                    match event {
                                        crate::sources::youtube::sabr::stream::SabrEvent::Stall => {
                                            debug!("YoutubeTrack: SABR stall detected for {}. Refreshing session...", id_rec);
                                            let sig_ts = cipher_rec.get_signature_timestamp().await.ok();
                                            // Find a client that can give SABR config
                                            let mut new_cfg_opt = None;
                                            for client in &clients_rec {
                                                if let Some(cfg) = client.get_sabr_config(&id_rec, visitor_data_rec.as_deref(), sig_ts, cipher_rec.clone(), 0).await {
                                                    new_cfg_opt = Some(cfg);
                                                    break;
                                                }
                                            }

                                            if let Some(new_cfg) = new_cfg_opt {
                                                debug!("YoutubeTrack: SABR session re-resolved for {}", id_rec);
                                                let po_token = new_cfg.po_token.as_deref().and_then(crate::sources::youtube::sabr::stream::decode_po_token);
                                                let _ = sabr_cmd_tx.send(crate::sources::youtube::sabr::stream::SabrCommand::UpdateSession {
                                                    server_abr_url: new_cfg.server_abr_url,
                                                    ustreamer_config: new_cfg.ustreamer_config,
                                                    po_token,
                                                    playback_cookie: None,
                                                });
                                            } else {
                                                error!("YoutubeTrack: SABR session re-resolution failed for {}", id_rec);
                                            }
                                        }
                                        crate::sources::youtube::sabr::stream::SabrEvent::Finished => {}
                                        crate::sources::youtube::sabr::stream::SabrEvent::Error(e) => {
                                            error!("YoutubeTrack: SABR stream error for {}: {}", id_rec, e);
                                        }
                                    }
                                }
                            }
                        }
                    });

                    Some(abort_tx)
                } else {
                    None
                };

                // Wait for commands or natural task completion
                loop {
                    tokio::select! {
                        cmd_res = cmd_rx.recv_async() => {
                            match cmd_res {
                                Ok(DecoderCommand::Seek(ms)) => {
                                    if is_sabr {
                                        current_seek_ms = ms;
                                        // Stop current processor and restart
                                        let _ = inner_cmd_tx.send(DecoderCommand::Stop);
                                        if let Some(tx) = sabr_abort_tx {
                                            let _ = tx.send(());
                                        }
                                        let _ = process_task.await;
                                        continue 'playback_loop;
                                    } else {
                                        let _ = inner_cmd_tx.send(DecoderCommand::Seek(ms));
                                    }
                                }
                                Ok(DecoderCommand::Stop) => {
                                    let _ = inner_cmd_tx.send(DecoderCommand::Stop);
                                    if let Some(tx) = sabr_abort_tx {
                                        let _ = tx.send(());
                                    }
                                    return; // Complete orchestrator
                                }
                                Err(_) => {
                                    // cmd_rx dropped, TrackHandle is gone
                                    let _ = inner_cmd_tx.send(DecoderCommand::Stop);
                                    if let Some(tx) = sabr_abort_tx {
                                        let _ = tx.send(());
                                    }
                                    return;
                                }
                            }
                        }
                        _ = &mut process_task => {
                            // AudioProcessor naturally finished (EOF or error)
                            if is_sabr {
                                // If SABR finished, we are done
                                return;
                            } else {
                                // For non-SABR, if it finished naturally, we are also done
                                return;
                            }
                        }
                    }
                }
            }
        });

        (rx, cmd_tx, err_rx, Some(opus_rx))
    }
}
