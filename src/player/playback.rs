use std::sync::{Arc, atomic::Ordering};

use tracing::{error, info};

use super::{context::PlayerContext, state::PlayerState};
use crate::{
    api,
    audio::playback::{PlaybackState, TrackHandle},
    server::Session,
};

pub async fn start_playback(
    player: &mut PlayerContext,
    track: String,
    session: Arc<Session>,
    source_manager: Arc<crate::sources::SourceManager>,
    lyrics_manager: Arc<crate::lyrics::LyricsManager>,
    routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    update_interval_secs: u64,
    user_data: Option<serde_json::Value>,
    end_time: Option<u64>,
) {
    if player.track.is_some() {
        let is_playing = if let Some(handle) = &player.track_handle {
            handle.get_state() != PlaybackState::Stopped
        } else {
            false
        };

        if is_playing {
            if let Some(track_data) = player.to_player_response().track {
                let end_event = api::OutgoingMessage::Event(api::LavalinkEvent::TrackEnd {
                    guild_id: player.guild_id.clone(),
                    track: track_data.clone(),
                    reason: api::TrackEndReason::Replaced,
                });
                session.send_message(&end_event).await;
            }
        }
    }

    if let Some(task) = player.track_task.take() {
        task.abort();
    }

    if let Some(handle) = &player.track_handle {
        player.stop_signal.store(true, Ordering::SeqCst);
        handle.stop();
    }

    {
        let engine = player.engine.lock().await;
        let mut mixer = engine.mixer.lock().await;
        mixer.stop_all();
    }

    player.track_info = api::tracks::Track::decode(&track);
    player.track = Some(track.clone());
    player.position = 0;
    player.paused = false;
    player.end_time = end_time; // set from request; None if not provided
    player.user_data = user_data.unwrap_or_else(|| serde_json::json!({}));
    player.stop_signal = Arc::new(std::sync::atomic::AtomicBool::new(false));

    // Decode the Rustalink track format to extract the actual playback URI
    let track_info = if let Some(decoded_track) = &player.track_info {
        decoded_track.info.clone()
    } else {
        // If decoding fails, treat the raw string as a direct identifier
        api::tracks::TrackInfo {
            title: "Unknown".to_string(),
            author: "Unknown".to_string(),
            length: 0,
            identifier: track.clone(),
            is_stream: false,
            uri: Some(track.clone()),
            artwork_url: None,
            isrc: None,
            source_name: "unknown".to_string(),
            is_seekable: true,
            position: 0,
        }
    };

    let identifier = track_info
        .uri
        .clone()
        .unwrap_or_else(|| track_info.identifier.clone());

    let playable_track = match source_manager
        .get_track(&track_info, routeplanner.clone())
        .await
    {
        Some(t) => t,
        None => {
            error!("Failed to resolve track: {}", identifier);
            if let Some(track_data) = player.to_player_response().track {
                let event = api::OutgoingMessage::Event(api::LavalinkEvent::TrackException {
                    guild_id: player.guild_id.clone(),
                    track: track_data.clone(),
                    exception: api::TrackException {
                        message: Some(format!("Failed to resolve track: {}", identifier)),
                        severity: crate::common::Severity::Common,
                        cause: format!("Failed to resolve track: {}", identifier),
                    },
                });
                session.send_message(&event).await;

                let end_event = api::OutgoingMessage::Event(api::LavalinkEvent::TrackEnd {
                    guild_id: player.guild_id.clone(),
                    track: track_data,
                    reason: api::TrackEndReason::LoadFailed,
                });
                session.send_message(&end_event).await;
            }
            return;
        }
    };

    // Attempt to fetch lyrics
    let lyrics_data_arc = player.lyrics_data.clone();
    let lyrics_manager_clone = lyrics_manager.clone();
    let track_info_clone = track_info.clone();
    let session_lyrics_clone = session.clone();
    let guild_id_lyrics = player.guild_id.clone();
    let lyrics_subscribed_on_start = player.lyrics_subscribed.clone();

    tokio::spawn(async move {
        if !lyrics_subscribed_on_start.load(Ordering::Relaxed) {
            return;
        }
        
        if let Some(lyrics) = lyrics_manager_clone.load_lyrics(&track_info_clone).await {
            {
                let mut lock = lyrics_data_arc.lock().await;
                *lock = Some(lyrics.clone());
            }

            let event = api::OutgoingMessage::Event(api::LavalinkEvent::LyricsFound {
                guild_id: guild_id_lyrics,
                lyrics: super::super::api::models::LavalinkLyrics {
                    source_name: track_info_clone.source_name.clone(),
                    provider: Some(lyrics.provider),
                    text: Some(lyrics.text),
                    lines: lyrics.lines.map(|lines| {
                        lines
                            .into_iter()
                            .map(|l| super::super::api::models::LavalinkLyricsLine {
                                timestamp: l.timestamp,
                                duration: Some(l.duration),
                                line: l.text,
                                plugin: serde_json::json!({}),
                            })
                            .collect()
                    }),
                    plugin: serde_json::json!({}),
                },
            });
            session_lyrics_clone.send_message(&event).await;
        } else {
            let event = api::OutgoingMessage::Event(api::LavalinkEvent::LyricsNotFound {
                guild_id: guild_id_lyrics,
            });
            session_lyrics_clone.send_message(&event).await;
        }
    });

    info!(
        "Playback starting: {} (source: {})",
        identifier, track_info.source_name
    );

    let (rx, cmd_tx, error_rx) = playable_track.start_decoding();
    let (handle, audio_state, vol, pos) = TrackHandle::new(cmd_tx, player.tape_stop.clone());

    {
        let engine = player.engine.lock().await;
        let mut mixer = engine.mixer.lock().await;
        mixer.add_track(rx, audio_state, vol, pos, player.config.clone());
    }

    player.track_handle = Some(handle.clone());

    let track_data = match player.to_player_response().track {
        Some(t) => t,
        None => {
            error!(
                "Failed to generate track response for guild {}",
                player.guild_id
            );
            return;
        }
    };
    let start_event = api::OutgoingMessage::Event(api::LavalinkEvent::TrackStart {
        guild_id: player.guild_id.clone(),
        track: track_data.clone(),
    });
    session.send_message(&start_event).await;

    let guild_id = player.guild_id.clone();
    let handle_clone = handle;
    let session_clone = session.clone();
    let stop_signal = player.stop_signal.clone();
    let track_data_clone = track_data.clone();
    let ping = player.ping.clone();
    let stuck_threshold_ms = player.config.stuck_threshold_ms;
    let lyrics_subscribed = player.lyrics_subscribed.clone();
    let lyrics_data = player.lyrics_data.clone();
    let last_lyric_index = player.last_lyric_index.clone();

    let track_task = tokio::spawn(async move {
        let mut state_interval = tokio::time::interval(std::time::Duration::from_millis(500));
    
        let update_every_n: u64 = (update_interval_secs * 2).max(1);
        let mut tick_count: u64 = 0;

        let mut last_position = handle_clone.get_position();
        let mut stuck_ms = 0u64;

        loop {
            state_interval.tick().await;
            tick_count = tick_count.wrapping_add(1);

            if stop_signal.load(Ordering::SeqCst) {
                break;
            }

            let current_state = handle_clone.get_state();
            if current_state == PlaybackState::Stopped {
                // Check if the decoder reported a fatal error.
                match error_rx.try_recv() {
                    Ok(err_msg) => {
                        tracing::warn!("[{}] Mid-playback decoder error: {}", guild_id, err_msg);
                        let exception_event =
                            api::OutgoingMessage::Event(api::LavalinkEvent::TrackException {
                                guild_id: guild_id.clone(),
                                track: track_data_clone.clone(),
                                exception: api::TrackException {
                                    message: Some(err_msg.clone()),
                                    severity: crate::common::Severity::Fault,
                                    cause: err_msg,
                                },
                            });
                        session_clone.send_message(&exception_event).await;

                        let end_event = api::OutgoingMessage::Event(api::LavalinkEvent::TrackEnd {
                            guild_id: guild_id.clone(),
                            track: track_data_clone.clone(),
                            reason: api::TrackEndReason::LoadFailed,
                        });
                        session_clone.send_message(&end_event).await;
                    }
                    Err(_) => {
                        // Channel empty or disconnected — normal natural finish.
                        let end_event = api::OutgoingMessage::Event(api::LavalinkEvent::TrackEnd {
                            guild_id: guild_id.clone(),
                            track: track_data_clone.clone(),
                            reason: api::TrackEndReason::Finished,
                        });
                        session_clone.send_message(&end_event).await;
                    }
                }
                break;
            }

            let current_pos = handle_clone.get_position();
            // Only check stuck when actively playing at normal rate.
            // During Stopping/Starting (tape effect) position is intentionally
            // frozen — skip the stuck check to avoid false TrackStuck events.
            let is_playing = current_state == PlaybackState::Playing;
            let is_transitioning = current_state == PlaybackState::Stopping
                || current_state == PlaybackState::Starting;

            if is_playing {
                if current_pos == last_position {
                    stuck_ms += 500;

                    // Give more time for the initial start (pos=0) to account for slow
                    // URL resolution and probing (~7-10s is common for YouTube).
                    let threshold = if current_pos == 0 {
                        stuck_threshold_ms.max(30000)
                    } else {
                        stuck_threshold_ms
                    };

                    if stuck_ms >= threshold {
                        let stuck_event =
                            api::OutgoingMessage::Event(api::LavalinkEvent::TrackStuck {
                                guild_id: guild_id.clone(),
                                track: track_data_clone.clone(),
                                threshold_ms: stuck_threshold_ms,
                            });
                        session_clone.send_message(&stuck_event).await;

                        tracing::warn!("Track {} got stuck!", track_data_clone.info.title);

                        // Send a playerUpdate immediately after stuck — mirrors Rustalink behaviour.
                        let stuck_update = api::OutgoingMessage::PlayerUpdate {
                            guild_id: guild_id.clone(),
                            state: PlayerState {
                                time: crate::server::now_ms(),
                                position: current_pos,
                                connected: true,
                                ping: ping.load(std::sync::atomic::Ordering::Relaxed),
                            },
                        };
                        session_clone.send_message(&stuck_update).await;
                    }
                } else {
                    stuck_ms = 0;
                }
            } else if !is_transitioning {
                // Paused / Stopped — also reset stuck counter
                stuck_ms = 0;
            }
            last_position = current_pos;

            // Fire PlayerUpdate every update_interval_secs (counted in 500ms ticks).
            if tick_count % update_every_n == 0 {
                let update = api::OutgoingMessage::PlayerUpdate {
                    guild_id: guild_id.clone(),
                    state: PlayerState {
                        time: crate::server::now_ms(),
                        position: current_pos,
                        connected: true,
                        ping: ping.load(std::sync::atomic::Ordering::Relaxed),
                    },
                };
                session_clone.send_message(&update).await;
            }

            // Lyrics synchronization
            if lyrics_subscribed.load(Ordering::Relaxed) {
                // Use try_lock to avoid blocking the 500ms tick if the lyrics
                // writer (fetcher task) currently holds the lock.
                if let Ok(lyrics_lock) = lyrics_data.try_lock() {
                    if let Some(lyrics) = &*lyrics_lock {
                        if let Some(lines) = &lyrics.lines {
                            let last_index = last_lyric_index.load(Ordering::Relaxed);

                            // Find the current target index based on position
                            let mut target_index = -1i64;
                            for (i, line) in lines.iter().enumerate() {
                                if current_pos >= line.timestamp {
                                    target_index = i as i64;
                                } else {
                                    break;
                                }
                            }

                            if target_index > last_index {
                                // Forward progression or jump
                                for i in (last_index + 1)..=target_index {
                                    let line = &lines[i as usize];
                                    let is_final = i == target_index;
                                    let event = api::OutgoingMessage::Event(
                                        api::LavalinkEvent::LyricsLine {
                                            guild_id: guild_id.clone(),
                                            line_index: i as i32,
                                            line: api::models::LavalinkLyricsLine {
                                                line: line.text.clone(),
                                                timestamp: line.timestamp,
                                                duration: Some(line.duration),
                                                plugin: serde_json::json!({}),
                                            },
                                            skipped: !is_final,
                                        },
                                    );
                                    session_clone.send_message(&event).await;
                                }
                                last_lyric_index.store(target_index, Ordering::SeqCst);
                            } else if target_index < last_index {
                                // Backward jump
                                if target_index != -1 {
                                    let line = &lines[target_index as usize];
                                    let event = api::OutgoingMessage::Event(
                                        api::LavalinkEvent::LyricsLine {
                                            guild_id: guild_id.clone(),
                                            line_index: target_index as i32,
                                            line: api::models::LavalinkLyricsLine {
                                                line: line.text.clone(),
                                                timestamp: line.timestamp,
                                                duration: Some(line.duration),
                                                plugin: serde_json::json!({}),
                                            },
                                            skipped: false,
                                        },
                                    );
                                    session_clone.send_message(&event).await;
                                }
                                last_lyric_index.store(target_index, Ordering::SeqCst);
                            }
                        }
                    }
                }
            }
        }
    });

    player.track_task = Some(track_task);
}
