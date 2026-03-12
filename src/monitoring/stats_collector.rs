use std::{process, sync::atomic::Ordering};

use sysinfo::{CpuRefreshKind, MemoryRefreshKind, ProcessRefreshKind, RefreshKind};

use crate::{
    protocol,
    server::{AppState, session::Session},
};

/// Collects system and process-level metrics.
pub fn collect_stats(app_state: &AppState, session: Option<&Session>) -> protocol::Stats {
    let mut system = app_state.system_state.lock();

    let pid = sysinfo::Pid::from_u32(process::id());
    let mut last_refresh = app_state.last_system_refresh.lock();

    if last_refresh.elapsed() >= std::time::Duration::from_secs(5) {
        system.refresh_specifics(
            RefreshKind::nothing()
                .with_cpu(CpuRefreshKind::nothing().with_cpu_usage())
                .with_memory(MemoryRefreshKind::nothing().with_ram()),
        );
        *last_refresh = std::time::Instant::now();
    }

    system.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing().with_memory(),
    );

    let cores = system.cpus().len() as u32;
    let logical_cores = cores.max(1);

    let lavalink_load = {
        let mut stat = app_state.process_stat.lock();
        stat.cpu().unwrap_or(0.0).clamp(0.0, logical_cores as f64) / logical_cores as f64
    };

    let process_used_memory = system.process(pid).map(|p| p.memory()).unwrap_or(0);

    let system_load = if system.cpus().is_empty() {
        0.0
    } else {
        (system.global_cpu_usage() as f64 / 100.0).clamp(0.0, 1.0)
    };

    let mut total_players = 0;
    let mut playing_players = 0;

    for session_entry in app_state.sessions.iter() {
        let session = session_entry.value();
        total_players += session.players.len() as u64;
        for player_entry in session.players.iter() {
            if let Ok(player) = player_entry.value().try_read()
                && player.is_playing()
            {
                playing_players += 1;
            }
        }
    }

    protocol::Stats {
        players: total_players,
        playing_players,
        uptime: app_state.start_time.elapsed().as_millis() as u64,
        memory: protocol::Memory {
            free: system.available_memory(),
            used: process_used_memory,
            allocated: process_used_memory,
            reservable: system.total_memory(),
        },
        cpu: protocol::Cpu {
            cores: cores as i32,
            system_load,
            lavalink_load,
        },
        frame_stats: session
            .and_then(|sess| FrameMetrics::calculate(sess, app_state.config.server.stats_interval)),
    }
}

struct FrameMetrics;

impl FrameMetrics {
    fn calculate(session: &Session, interval_secs: u64) -> Option<protocol::FrameStats> {
        let mut historical_sent = session.total_sent_historical.load(Ordering::Acquire);
        let mut historical_nulled = session.total_nulled_historical.load(Ordering::Acquire);
        let mut active_player_count = 0;

        for entry in session.players.iter() {
            if let Ok(player) = entry.value().try_read()
                && player.track.is_some()
                && !player.paused
            {
                active_player_count += 1;
                historical_sent += player.frames_sent.load(Ordering::Acquire);
                historical_nulled += player.frames_nulled.load(Ordering::Acquire);
            }
        }

        let last_sent = session
            .last_stats_sent
            .swap(historical_sent, Ordering::AcqRel);
        let last_nulled = session
            .last_stats_nulled
            .swap(historical_nulled, Ordering::AcqRel);

        if active_player_count == 0 || (last_sent == 0 && last_nulled == 0) {
            return None;
        }

        let delta_sent = historical_sent.saturating_sub(last_sent) as i32;
        let delta_nulled = historical_nulled.saturating_sub(last_nulled) as i32;

        // Standard 50 fps per active player
        let expected_total = (interval_secs * 50) as i32 * active_player_count;
        let deficit = expected_total - (delta_sent + delta_nulled);

        Some(protocol::FrameStats {
            sent: delta_sent / active_player_count,
            nulled: delta_nulled / active_player_count,
            deficit: deficit / active_player_count,
        })
    }
}
