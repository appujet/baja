use std::sync::atomic::{AtomicU64, Ordering};

use crate::{api, server::AppState};

pub fn collect_stats(state: &AppState, uptime: u64) -> api::Stats {
    let mut total_players = 0i32;
    let mut playing_players = 0i32;

    let mut total_sent = 0;
    let mut total_nulled = 0;
    let mut player_count = 0;

    for session in state.sessions.iter() {
        for player in session.players.iter() {
            total_players += 1;
            if player.track.is_some() && !player.paused {
                playing_players += 1;

                player_count += 1;
                total_sent += player
                    .frames_sent
                    .swap(0, std::sync::atomic::Ordering::Relaxed)
                    as i32;
                total_nulled += player
                    .frames_nulled
                    .swap(0, std::sync::atomic::Ordering::Relaxed)
                    as i32;
            }
        }
    }

    let frame_stats = if player_count != 0 {
        let total_deficit = player_count * 3000 - (total_sent + total_nulled); // 3000 per minute per player
        Some(api::FrameStats {
            sent: total_sent / player_count,
            nulled: total_nulled / player_count,
            deficit: total_deficit / player_count,
        })
    } else {
        None
    };

    let (mem_used, mem_free, mem_total) = read_memory_stats();

    let system_load = read_system_load();

    api::Stats {
        players: total_players,
        playing_players,
        uptime,
        memory: api::Memory {
            free: mem_free,
            used: mem_used,
            allocated: mem_used,
            reservable: mem_total,
        },
        cpu: api::Cpu {
            cores: num_cpus(),
            system_load,
            lavalink_load: read_process_cpu_load(),
        },
        frame_stats,
    }
}

fn read_system_load() -> f64 {
    std::fs::read_to_string("/proc/loadavg")
        .ok()
        .and_then(|s| {
            s.split_whitespace()
                .next()
                .and_then(|v| v.parse::<f64>().ok())
        })
        .unwrap_or(0.0)
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

    (d_cpu / d_wall).clamp(0.0, 1.0)
}
