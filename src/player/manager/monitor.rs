use std::sync::{Arc, atomic::Ordering};

use tracing::warn;

use super::lyrics::sync_lyrics;
use crate::{
    audio::playback::{PlaybackState, TrackHandle},
    common::types::GuildId,
    player::state::PlayerState,
    protocol::{
        self,
        events::{RustalinkEvent, TrackEndReason, TrackException},
        models::LyricsData,
        tracks::Track,
    },
    server::Session,
};

pub struct MonitorCtx {
    pub guild_id: GuildId,
    pub handle: TrackHandle,
    pub err_rx: flume::Receiver<String>,
    pub session: Arc<Session>,
    pub track: Track,
    pub stop_signal: Arc<std::sync::atomic::AtomicBool>,
    pub ping: Arc<std::sync::atomic::AtomicI64>,
    pub stuck_threshold_ms: u64,
    pub update_every_n: u64,
    pub lyrics_subscribed: Arc<std::sync::atomic::AtomicBool>,
    pub lyrics_data: Arc<tokio::sync::Mutex<Option<LyricsData>>>,
    pub last_lyric_index: Arc<std::sync::atomic::AtomicI64>,
}

pub async fn monitor_loop(ctx: MonitorCtx) {
    let MonitorCtx {
        guild_id,
        handle,
        err_rx,
        session,
        track,
        stop_signal,
        ping,
        stuck_threshold_ms,
        update_every_n,
        lyrics_subscribed,
        lyrics_data,
        last_lyric_index,
    } = ctx;

    let mut interval = tokio::time::interval(std::time::Duration::from_millis(500));
    let mut tick: u64 = 0;
    let mut last_pos = handle.get_position();
    // Wall-clock instant that the position last changed — used for stuck detection.
    // This avoids false positives from integer truncation of position in ms.
    let mut last_pos_changed_at = std::time::Instant::now();
    let mut stuck_fired = false;

    loop {
        interval.tick().await;
        tick = tick.wrapping_add(1);

        if stop_signal.load(Ordering::SeqCst) {
            break;
        }

        let state = handle.get_state();

        // -- Track ended --------------------------------------------------
        if state == PlaybackState::Stopped {
            let reason = match err_rx.try_recv() {
                Ok(err) => {
                    warn!("[{}] mid-playback decoder error: {}", guild_id, err);
                    session.send_message(&protocol::OutgoingMessage::Event {
                        event: RustalinkEvent::TrackException {
                            guild_id: guild_id.clone(),
                            track: track.clone(),
                            exception: TrackException {
                                message: Some(err.clone()),
                                severity: crate::common::Severity::Fault,
                                cause: err.clone(),
                                cause_stack_trace: Some(err),
                            },
                        },
                    });
                    TrackEndReason::LoadFailed
                }
                Err(_) => TrackEndReason::Finished,
            };

            session.send_message(&protocol::OutgoingMessage::Event {
                event: RustalinkEvent::TrackEnd {
                    guild_id: guild_id.clone(),
                    track,
                    reason,
                },
            });

            // Clear track state in PlayerContext if it hasn't been replaced.
            if let Some(player_arc) = session.players.get(&guild_id).map(|kv| kv.value().clone()) {
                let mut p = player_arc.write().await;
                if p.track_handle
                    .as_ref()
                    .map(|h| h.is_same(&handle))
                    .unwrap_or(false)
                {
                    p.track = None;
                    p.track_info = None;
                    p.track_handle = None;
                }
            }
            break;
        }

        let cur_pos = handle.get_position();

        if state == PlaybackState::Playing {
            if cur_pos != last_pos {
                last_pos_changed_at = std::time::Instant::now();
                stuck_fired = false;
            } else if !stuck_fired {
                let effective_threshold = if cur_pos == 0 {
                    stuck_threshold_ms.max(30_000)
                } else {
                    stuck_threshold_ms
                };

                let elapsed_ms = last_pos_changed_at.elapsed().as_millis() as u64;
                if elapsed_ms >= effective_threshold {
                    session.send_message(&protocol::OutgoingMessage::Event {
                        event: RustalinkEvent::TrackStuck {
                            guild_id: guild_id.clone(),
                            track: track.clone(),
                            threshold_ms: stuck_threshold_ms,
                        },
                    });
                    warn!(
                        "[{}] Track stuck: position stalled at {}ms for {}ms (threshold {}ms)",
                        guild_id, cur_pos, elapsed_ms, stuck_threshold_ms
                    );
                    stuck_fired = true;
                    handle.stop();
                }
            }
        } else {
            last_pos_changed_at = std::time::Instant::now();
        }

        last_pos = cur_pos;

        // -- PlayerUpdate --------------------------------------------------
        if tick % update_every_n == 0 {
            session.send_message(&protocol::OutgoingMessage::PlayerUpdate {
                guild_id: guild_id.clone(),
                state: PlayerState {
                    time: crate::common::utils::now_ms(),
                    position: cur_pos,
                    connected: true,
                    ping: ping.load(Ordering::Relaxed),
                },
            });
        }

        // -- Lyrics sync ---------------------------------------------------
        if lyrics_subscribed.load(Ordering::Relaxed) {
            sync_lyrics(
                &guild_id,
                cur_pos,
                &last_lyric_index,
                &lyrics_data,
                &session,
            )
            .await;
        }
    }
}
