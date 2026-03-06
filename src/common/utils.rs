use std::{
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

static RESOURCE_CACHE: StdMutex<Option<(String, Instant)>> = StdMutex::new(None);

/// Queries the program's current memory profile.
pub fn memory_usage_report() -> String {
    let mut guard = RESOURCE_CACHE.lock().unwrap();

    if let Some((report, timestamp)) = guard.as_ref()
        && timestamp.elapsed().as_secs() < 1
    {
        return report.clone();
    }

    let mut system = sysinfo::System::new();
    let pid = sysinfo::Pid::from_u32(std::process::id());
    system.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::Some(&[pid]),
        true,
        sysinfo::ProcessRefreshKind::nothing().with_memory(),
    );

    let bytes = system.process(pid).map(|p| p.memory()).unwrap_or(0);
    let formatted = format_byte_size(bytes);

    *guard = Some((formatted.clone(), Instant::now()));
    formatted
}

fn format_byte_size(bytes: u64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut index = 0;

    while size >= 1024.0 && index < units.len() - 1 {
        size /= 1024.0;
        index += 1;
    }

    format!("{:.2} {}", size, units[index])
}

pub const DEFAULT_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.0.0 Safari/537.36";

/// Returns the default User-Agent string.
pub fn default_user_agent() -> String {
    DEFAULT_USER_AGENT.to_string()
}
