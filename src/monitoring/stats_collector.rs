use crate::server::AppState;
use crate::api;

pub fn collect_stats(state: &AppState, uptime: u64) -> api::Stats {
    let mut total_players = 0i32;
    let mut playing_players = 0i32;

    for session in state.sessions.iter() {
        for player in session.players.iter() {
            total_players += 1;
            if player.track.is_some() && !player.paused {
                playing_players += 1;
            }
        }
    }

    let (mem_used, mem_free, mem_total) = read_memory_stats();

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
            system_load: 0.0,
            lavalink_load: 0.0,
        },
        frame_stats: None,
    }
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
