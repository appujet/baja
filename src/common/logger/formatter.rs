use core::fmt as core_fmt;
use std::fs;

use tracing::{Event, Subscriber};
use tracing_subscriber::{
    fmt::{
        self, FmtContext,
        format::{FormatEvent, FormatFields},
    },
    registry::LookupSpan,
};

pub fn get_ram_usage() -> String {
    if let Ok(status) = fs::read_to_string("/proc/self/status") {
        for line in status.lines() {
            if line.starts_with("VmRSS:") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    if let Ok(rss_kb) = parts[1].parse::<u64>() {
                        let rss_f = rss_kb as f64;
                        if rss_f < 1024.0 {
                            return format!("{:.2} KB", rss_f);
                        } else if rss_f < 1024.0 * 1024.0 {
                            return format!("{:.2} MB", rss_f / 1024.0);
                        } else {
                            return format!("{:.2} GB", rss_f / (1024.0 * 1024.0));
                        }
                    }
                }
            }
        }
    }
    "0.00 KB".to_string()
}

pub struct CustomFormatter {
    use_ansi: bool,
}

impl CustomFormatter {
    pub fn new(use_ansi: bool) -> Self {
        Self { use_ansi }
    }
}

impl<S, N> FormatEvent<S, N> for CustomFormatter
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: fmt::format::Writer<'_>,
        event: &Event<'_>,
    ) -> core_fmt::Result {
        let reset = if self.use_ansi { "\x1b[0m" } else { "" };
        let bold = if self.use_ansi { "\x1b[1m" } else { "" };
        let dim = if self.use_ansi { "\x1b[2m" } else { "" };

        // RAM Usage
        write!(writer, "{}[{}]{} ", dim, get_ram_usage(), reset)?;

        // Timestamp
        let format = time::macros::format_description!(
            "[year]-[month]-[day] [hour]:[minute]:[second].[subsecond digits:3]"
        );
        let now =
            time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
        let timestamp = now
            .format(&format)
            .unwrap_or_else(|_| "Unknown Time".to_string());

        if self.use_ansi {
            write!(writer, "{}[{}]{} ", dim, timestamp, reset)?;
        } else {
            write!(writer, "[{}] ", timestamp)?;
        }

        // Level
        let metadata = event.metadata();
        let level = metadata.level();
        let level_str = format!("{: <5}", level.to_string());

        if self.use_ansi {
            let level_color = match *level {
                tracing::Level::ERROR => "\x1b[31m", // Red
                tracing::Level::WARN => "\x1b[33m",  // Yellow
                tracing::Level::INFO => "\x1b[32m",  // Green
                tracing::Level::DEBUG => "\x1b[34m", // Blue
                tracing::Level::TRACE => "\x1b[35m", // Magenta
            };
            write!(writer, "{}{}{}{} ", level_color, bold, level_str, reset)?;
        } else {
            write!(writer, "{} ", level_str)?;
        }

        // Thread ID
        let thread_id_full = format!("{:?}", std::thread::current().id());
        let id_num = thread_id_full.replace("ThreadId(", "").replace(")", "");
        write!(writer, "ThreadId({}) ", id_num)?;

        // Target and Line
        let target = metadata.target();
        let line = metadata
            .line()
            .map(|l| l.to_string())
            .unwrap_or_else(|| "??".to_string());
        write!(writer, "{}{}: {}{} ", dim, target, line, reset)?;

        // Message separator
        write!(writer, "> ")?;

        // Message
        ctx.format_fields(writer.by_ref(), event)?;

        // Final reset to prevent any leakage into the terminal shell
        write!(writer, "{}", reset)?;

        writeln!(writer)
    }
}
