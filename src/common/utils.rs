use std::{
    fs,
    sync::Mutex as StdMutex,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

// ANSI Color Codes
pub const RESET: &str = "\x1b[0m";
pub const BOLD: &str = "\x1b[1m";
pub const DIM: &str = "\x1b[2m";

pub const ORANGE: &str = "\x1b[38;5;208m";
pub const GREEN: &str = "\x1b[32m";
pub const CYAN: &str = "\x1b[36m";
pub const YELLOW: &str = "\x1b[33m";
pub const BLUE: &str = "\x1b[34m";
pub const MAGENTA: &str = "\x1b[35m";
pub const RED: &str = "\x1b[31m";

// Log Level Colors
pub const COLOR_ERROR: &str = RED;
pub const COLOR_WARN: &str = YELLOW;
pub const COLOR_INFO: &str = GREEN;
pub const COLOR_DEBUG: &str = BLUE;
pub const COLOR_TRACE: &str = MAGENTA;

/// Returns the current time in milliseconds since the Unix epoch.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Simple ANSI stripper to prevent the log file from being polluted with escape sequences.
pub fn strip_ansi_escapes(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if let Some('[') = chars.peek() {
                chars.next();
                while let Some(&nc) = chars.peek() {
                    chars.next();
                    if nc.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else {
            result.push(c);
        }
    }
    result
}

static RAM_CACHE: StdMutex<Option<(String, Instant)>> = StdMutex::new(None);

/// Returns the current RAM usage of the process, cached for 1 second.
pub fn get_ram_usage() -> String {
    let mut cache = RAM_CACHE.lock().unwrap();
    if let Some((val, _)) = cache
        .as_ref()
        .filter(|(_, last_update)| last_update.elapsed().as_secs() < 1)
    {
        return val.clone();
    }

    let val = read_proc_self_status().unwrap_or_else(|| "0.00 KB".to_string());
    *cache = Some((val.clone(), Instant::now()));
    val
}

fn read_proc_self_status() -> Option<String> {
    let status = fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if let Some(rss_kb) = line
            .strip_prefix("VmRSS:")
            .and_then(|s| s.split_whitespace().next())
            .and_then(|s| s.parse::<u64>().ok())
        {
            let rss_f = rss_kb as f64;
            return Some(if rss_f < 1024.0 {
                format!("{:.2} KB", rss_f)
            } else if rss_f < 1024.0 * 1024.0 {
                format!("{:.2} MB", rss_f / 1024.0)
            } else {
                format!("{:.2} GB", rss_f / (1024.0 * 1024.0))
            });
        }
    }
    None
}

pub const DEFAULT_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.0.0 Safari/537.36";

/// Returns the default User-Agent string.
pub fn default_user_agent() -> String {
    DEFAULT_USER_AGENT.to_string()
}
