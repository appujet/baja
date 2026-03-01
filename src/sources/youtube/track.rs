use std::{net::IpAddr, sync::Arc};

use flume::{Receiver, Sender};
use tracing::{debug, error, info, warn};

use crate::{
    audio::processor::DecoderCommand,
    configs::HttpProxyConfig,
    sources::{
        plugin::PlayableTrack,
        youtube::{
            cipher::YouTubeCipherManager,
            clients::YouTubeClient,
            oauth::YouTubeOAuth,
            sabr::SabrConfig,
            utils::{create_reader, detect_audio_kind},
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
    Url(String, String),
    Sabr(SabrConfig, String, Arc<dyn YouTubeClient>),
}

impl PlayableTrack for YoutubeTrack {
    fn start_decoding(
        &self,
        config: crate::configs::player::PlayerConfig,
    ) -> (
        Receiver<crate::audio::PooledBuffer>,
        Sender<DecoderCommand>,
        flume::Receiver<String>,
        Option<Receiver<std::sync::Arc<Vec<u8>>>>,
    ) {
        let (tx, rx) = flume::bounded((config.buffer_duration_ms / 20) as usize);
        let (cmd_tx, cmd_rx) = flume::bounded(8);
        let (err_tx, err_rx) = flume::bounded(1);

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
                let signature_timestamp = cipher_manager_async.get_signature_timestamp().await.ok();
                let mut resolved = None;

                for client in &clients_async {
                    let client_name = client.name().to_string();

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
                                    Some(crate::common::types::AudioFormat::Webm)
                                } else if m.contains("mp4") {
                                    Some(crate::common::types::AudioFormat::Mp4)
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
                            let url_clone = url.clone();
                            let cipher_clone = cipher_manager_async.clone();
                            let proxy_clone = proxy_bg.clone();
                            let client_name_inner = client_name.clone();

                            let reader_res = tokio::task::spawn_blocking(move || {
                                create_reader(
                                    &url_clone,
                                    &client_name_inner,
                                    local_addr_bg,
                                    proxy_clone,
                                    cipher_clone,
                                )
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

                            let kind = detect_audio_kind(&url, is_hls);
                            (reader, Some(kind), client_name, None, None)
                        }
                    };

                let (inner_cmd_tx, inner_cmd_rx) = flume::bounded(8);
                let tx_clone = tx.clone();
                let err_tx_clone = err_tx.clone();

                let config_for_processor = config.clone();
                let mut process_task = tokio::task::spawn_blocking(move || {
                    match crate::audio::processor::AudioProcessor::new(
                        reader,
                        kind,
                        tx_clone,
                        inner_cmd_rx,
                        Some(err_tx_clone.clone()),
                        config_for_processor,
                    ) {
                        Ok(mut processor) => {
                            if let Err(e) = processor.run() {
                                error!("YoutubeTrack: Decoding session finished with error: {}", e);
                            }
                        }
                        Err(e) => {
                            error!("YoutubeTrack: AudioProcessor initialization failed: {}", e);
                            let _ =
                                err_tx_clone.send(format!("Failed to initialize processor: {}", e));
                        }
                    }
                });

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
                            _ = abort_rx => {}
                            event_opt = sabr_event_rx.recv_async() => {
                                if let Ok(event) = event_opt {
                                    match event {
                                        crate::sources::youtube::sabr::stream::SabrEvent::Stall => {
                                            debug!("YoutubeTrack: SABR stall detected for {}. Refreshing session...", id_rec);
                                            let sig_ts = cipher_rec.get_signature_timestamp().await.ok();
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

                loop {
                    tokio::select! {
                        cmd_res = cmd_rx.recv_async() => {
                            match cmd_res {
                                Ok(DecoderCommand::Seek(ms)) => {
                                    if is_sabr {
                                        current_seek_ms = ms;
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
                                    return;
                                }
                                Err(_) => {
                                    let _ = inner_cmd_tx.send(DecoderCommand::Stop);
                                    if let Some(tx) = sabr_abort_tx {
                                        let _ = tx.send(());
                                    }
                                    return;
                                }
                            }
                        }
                        _ = &mut process_task => {
                            return;
                        }
                    }
                }
            }
        });

        (rx, cmd_tx, err_rx, None)
    }
}
