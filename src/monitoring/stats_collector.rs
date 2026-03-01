use std::sync::atomic::{AtomicU64, Ordering};

use crate::{protocol, server::AppState};

pub fn collect_stats(
    state: &AppState,
    session: Option<&crate::server::session::Session>,
) -> protocol::Stats {
    let uptime = state.start_time.elapsed().as_millis() as u64;

    // --- Compute live player counts across ALL sessions (same as Lavalink) ---
    let mut total_players: i32 = 0;
    let mut playing_players: i32 = 0;

    for entry in state.sessions.iter() {
        let s = entry.value();
        total_players += s.players.len() as i32;
        for player_ref in s.players.iter() {
            if let Ok(p) = player_ref.value().try_read() {
                if p.track.is_some() && !p.paused {
                    playing_players += 1;
                }
            }
        }
    }
    // Also count resumable sessions
    for entry in state.resumable_sessions.iter() {
        let s = entry.value();
        total_players += s.players.len() as i32;
        for player_ref in s.players.iter() {
            if let Ok(p) = player_ref.value().try_read() {
                if p.track.is_some() && !p.paused {
                    playing_players += 1;
                }
            }
        }
    }

    // --- Frame stats: per-session only, over actively playing players ---
    let frame_stats = if let Some(s) = session {
        let mut current_total_sent: u64 = s.total_sent_historical.load(Ordering::Relaxed);
        let mut current_total_nulled: u64 = s.total_nulled_historical.load(Ordering::Relaxed);
        let mut player_count: i32 = 0;

        let arcs: Vec<_> = s.players.iter().map(|kv| kv.value().clone()).collect();
        for arc in arcs {
            if let Ok(player) = arc.try_read() {
                if player.track.is_some() && !player.paused {
                    player_count += 1;
                    current_total_sent += player.frames_sent.load(Ordering::Relaxed);
                    current_total_nulled += player.frames_nulled.load(Ordering::Relaxed);
                }
            }
        }

        // Delta since last stats call
        let last_sent = s
            .last_stats_sent
            .swap(current_total_sent, Ordering::Relaxed);
        let last_nulled = s
            .last_stats_nulled
            .swap(current_total_nulled, Ordering::Relaxed);

        let (total_sent, total_nulled) = if last_sent != 0 || last_nulled != 0 {
            (
                current_total_sent.saturating_sub(last_sent) as i32,
                current_total_nulled.saturating_sub(last_nulled) as i32,
            )
        } else {
            (0, 0)
        };

        if player_count != 0 {
            let expected_per_player = (state.config.server.stats_interval * 50) as i32;
            let total_deficit = player_count * expected_per_player - (total_sent + total_nulled);

            Some(protocol::FrameStats {
                sent: total_sent / player_count,
                nulled: total_nulled / player_count,
                deficit: total_deficit / player_count,
            })
        } else {
            None
        }
    } else {
        None
    };

    let (mem_used, _mem_free, mem_total) = read_memory_stats();

    let cores = num_cpus();
    let system_load = read_system_load();
    let lavalink_load = (read_process_cpu_load() / cores as f64).clamp(0.0, 1.0);

    protocol::Stats {
        players: total_players,
        playing_players,
        uptime,
        memory: protocol::Memory {
            free: mem_total.saturating_sub(mem_used),
            used: mem_used,
            allocated: mem_used,
            reservable: mem_total,
        },
        cpu: protocol::Cpu {
            cores,
            system_load,
            lavalink_load,
        },
        frame_stats,
    }
}

fn read_system_load() -> f64 {
    static PREV_IDLE: AtomicU64 = AtomicU64::new(0);
    static PREV_TOTAL: AtomicU64 = AtomicU64::new(0);

    let stat = match std::fs::read_to_string("/proc/stat") {
        Ok(s) => s,
        Err(_) => return 0.0,
    };

    let first_line = stat.lines().next().unwrap_or("");
    let parts: Vec<&str> = first_line.split_whitespace().collect();
    if parts.len() < 5 || parts[0] != "cpu" {
        return 0.0;
    }

    let mut total: u64 = 0;
    for part in &parts[1..] {
        total += part.parse::<u64>().unwrap_or(0);
    }
    // Idle is field 4 (0-indexed)
    let idle = parts[4].parse::<u64>().unwrap_or(0);

    let prev_idle = PREV_IDLE.swap(idle, Ordering::Relaxed);
    let prev_total = PREV_TOTAL.swap(total, Ordering::Relaxed);

    if prev_total == 0 {
        return 0.0;
    }

    let d_idle = idle.saturating_sub(prev_idle);
    let d_total = total.saturating_sub(prev_total);

    if d_total == 0 {
        return 0.0;
    }

    (d_total.saturating_sub(d_idle)) as f64 / d_total as f64
}

fn num_cpus() -> i32 {
    std::thread::available_parallelism()
        .map(|n| n.get() as i32)
        .unwrap_or(1)
}

fn read_memory_stats() -> (u64, u64, u64) {
    let rss = std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmRSS:"))
                .and_then(|l| {
                    l.split_whitespace()
                        .nth(1)
                        .and_then(|v| v.parse::<u64>().ok())
                })
                .map(|kb| kb * 1024)
        })
        .unwrap_or(0);

    let (mut total, mut free) = (0u64, 0u64);
    if let Ok(meminfo) = std::fs::read_to_string("/proc/meminfo") {
        for line in meminfo.lines() {
            if line.starts_with("MemTotal:") {
                total = line
                    .split_whitespace()
                    .nth(1)
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(0)
                    * 1024;
            } else if line.starts_with("MemAvailable:") {
                free = line
                    .split_whitespace()
                    .nth(1)
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(0)
                    * 1024;
            }
        }
    }
    (rss, free, total)
}

/// Reads per-process CPU time from `/proc/self/stat` and computes the CPU
/// load fraction since the last call. Returns a value in `[0.0, 1.0]`
/// representing fraction of total CPU time used by this process.
///
/// Linux kernel always uses 100 ticks/sec for USER_HZ in /proc/self/stat,
/// so we avoid a libc dependency by hardcoding 100.
fn read_process_cpu_load() -> f64 {
    static PREV_CPU: AtomicU64 = AtomicU64::new(0);
    static PREV_WALL: AtomicU64 = AtomicU64::new(0);

    // Read /proc/self/stat — utime is field 14, stime is field 15 (1-indexed).
    // The comm field (2nd) can contain spaces and parens; skip past the closing ')'.
    let stat = match std::fs::read_to_string("/proc/self/stat") {
        Ok(s) => s,
        Err(_) => return 0.0,
    };
    let after_comm = match stat.rfind(')') {
        Some(i) => &stat[i + 1..],
        None => return 0.0,
    };

    let fields: Vec<&str> = after_comm.split_whitespace().collect();
    // After ')': state(0), ppid(1), pgrp(2), session(3), tty(4), tpgid(5),
    //             flags(6), minflt(7), cminflt(8), majflt(9), cmajflt(10),
    //             utime(11), stime(12), ...
    let utime: u64 = fields.get(11).and_then(|v| v.parse().ok()).unwrap_or(0);
    let stime: u64 = fields.get(12).and_then(|v| v.parse().ok()).unwrap_or(0);
    let cpu_ticks = utime + stime;

    // Wall-clock in ticks: uptime_secs * USER_HZ (always 100 on Linux)
    let uptime_sec: f64 = std::fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|s| s.split_whitespace().next().and_then(|v| v.parse().ok()))
        .unwrap_or(0.0);

    const USER_HZ: u64 = 100;
    let wall_ticks = (uptime_sec * USER_HZ as f64) as u64;

    let prev_cpu = PREV_CPU.swap(cpu_ticks, Ordering::Relaxed);
    let prev_wall = PREV_WALL.swap(wall_ticks, Ordering::Relaxed);

    // First call: no delta yet
    if prev_wall == 0 {
        return 0.0;
    }

    let d_cpu = cpu_ticks.saturating_sub(prev_cpu) as f64;
    let d_wall = wall_ticks.saturating_sub(prev_wall) as f64;

    if d_wall == 0.0 {
        return 0.0;
    }

    d_cpu / d_wall
}
