use std::sync::{Arc, atomic::Ordering};

use crate::{
    common::types::GuildId,
    protocol::{
        self,
        events::RustalinkEvent,
        models::{LyricsData, RustalinkLyrics, RustalinkLyricsLine},
        tracks::TrackInfo,
    },
    server::Session,
};

/// Spawn a non-blocking task that fetches lyrics and sends the result.
pub fn spawn_lyrics_fetch(
    subscribed: Arc<std::sync::atomic::AtomicBool>,
    lyrics_data: Arc<tokio::sync::Mutex<Option<LyricsData>>>,
    track_info: TrackInfo,
    lyrics_manager: Arc<crate::lyrics::LyricsManager>,
    session: Arc<Session>,
    guild_id: GuildId,
) {
    tokio::spawn(async move {
        if !subscribed.load(Ordering::Relaxed) {
            return;
        }

        let event = if let Some(lyrics) = lyrics_manager.load_lyrics(&track_info).await {
            let mut lock = lyrics_data.lock().await;
            *lock = Some(lyrics.clone());

            protocol::OutgoingMessage::Event {
                event: RustalinkEvent::LyricsFound {
                    guild_id,
                    lyrics: RustalinkLyrics {
                        source_name: track_info.source_name,
                        provider: Some(lyrics.provider),
                        text: Some(lyrics.text),
                        lines: lyrics.lines.map(|lines| {
                            lines
                                .into_iter()
                                .map(|l| RustalinkLyricsLine {
                                    timestamp: l.timestamp,
                                    duration: Some(l.duration),
                                    line: l.text,
                                    plugin: serde_json::json!({}),
                                })
                                .collect()
                        }),
                        plugin: serde_json::json!({}),
                    },
                },
            }
        } else {
            protocol::OutgoingMessage::Event {
                event: RustalinkEvent::LyricsNotFound { guild_id },
            }
        };

        session.send_message(&event);
    });
}

/// Emit the current lyrics line(s) based on playback position.
pub async fn sync_lyrics(
    guild_id: &GuildId,
    pos_ms: u64,
    last_idx: &Arc<std::sync::atomic::AtomicI64>,
    lyrics_data: &Arc<tokio::sync::Mutex<Option<LyricsData>>>,
    session: &Session,
) {
    let Ok(lock) = lyrics_data.try_lock() else {
        return;
    };
    let Some(lyrics) = &*lock else { return };
    let Some(lines) = &lyrics.lines else { return };

    // Find the highest line index whose timestamp <= current position.
    let target = lines
        .iter()
        .rposition(|l| pos_ms >= l.timestamp)
        .map(|i| i as i64)
        .unwrap_or(-1);

    let last = last_idx.load(Ordering::Relaxed);
    if target == last {
        return;
    }

    if target > last {
        // Forward: emit any lines we skipped.
        for i in (last + 1)..=target {
            let line = &lines[i as usize];
            session.send_message(&protocol::OutgoingMessage::Event {
                event: RustalinkEvent::LyricsLine {
                    guild_id: guild_id.clone(),
                    line_index: i as i32,
                    line: RustalinkLyricsLine {
                        line: line.text.clone(),
                        timestamp: line.timestamp,
                        duration: Some(line.duration),
                        plugin: serde_json::json!({}),
                    },
                    skipped: i != target,
                },
            });
        }
    } else if target >= 0 {
        // Backward seek - emit the new current line.
        let line = &lines[target as usize];
        session.send_message(&protocol::OutgoingMessage::Event {
            event: RustalinkEvent::LyricsLine {
                guild_id: guild_id.clone(),
                line_index: target as i32,
                line: RustalinkLyricsLine {
                    line: line.text.clone(),
                    timestamp: line.timestamp,
                    duration: Some(line.duration),
                    plugin: serde_json::json!({}),
                },
                skipped: false,
            },
        });
    }

    last_idx.store(target, Ordering::SeqCst);
}
