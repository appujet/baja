use core::fmt::{self as core_fmt};

use tracing::{Event, Level, Subscriber};
use tracing_subscriber::{
    fmt::{
        self, FmtContext,
        format::{FormatEvent, FormatFields},
    },
    registry::LookupSpan,
};

use crate::common::utils::{
    BOLD, COLOR_DEBUG, COLOR_ERROR, COLOR_INFO, COLOR_TRACE, COLOR_WARN, DIM, RESET, get_ram_usage,
};

pub struct CustomFormatter {
    use_ansi: bool,
}

impl CustomFormatter {
    pub fn new(use_ansi: bool) -> Self {
        Self { use_ansi }
    }

    fn write_timestamp(&self, writer: &mut fmt::format::Writer<'_>) -> core_fmt::Result {
        let format = time::macros::format_description!(
            "[year]-[month]-[day] [hour]:[minute]:[second].[subsecond digits:3]"
        );
        let now =
            time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
        let timestamp = now
            .format(&format)
            .unwrap_or_else(|_| "Unknown Time".to_string());

        if self.use_ansi {
            write!(writer, "{DIM}[{timestamp}]{RESET} ")
        } else {
            write!(writer, "[{timestamp}] ")
        }
    }

    fn write_level(&self, writer: &mut fmt::format::Writer<'_>, level: &Level) -> core_fmt::Result {
        let level_str = format!("{: <5}", level.to_string());

        if self.use_ansi {
            let color = match *level {
                Level::ERROR => COLOR_ERROR,
                Level::WARN => COLOR_WARN,
                Level::INFO => COLOR_INFO,
                Level::DEBUG => COLOR_DEBUG,
                Level::TRACE => COLOR_TRACE,
            };
            write!(writer, "{color}{BOLD}{level_str}{RESET} ")
        } else {
            write!(writer, "{level_str} ")
        }
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
        let (reset, dim) = if self.use_ansi {
            (RESET, DIM)
        } else {
            ("", "")
        };

        // RAM Usage
        write!(writer, "{dim}[{}]{reset} ", get_ram_usage())?;

        // Timestamp
        self.write_timestamp(&mut writer)?;

        // Level
        let metadata = event.metadata();
        self.write_level(&mut writer, metadata.level())?;

        // Target and Line
        let target = metadata.target();
        let line = metadata
            .line()
            .map(|l| l.to_string())
            .unwrap_or_else(|| "??".to_string());
        write!(writer, "{dim}{target}: {line}{reset} > ")?;

        // Message
        ctx.format_fields(writer.by_ref(), event)?;

        // Final reset and newline
        write!(writer, "{reset}")?;
        writeln!(writer)
    }
}
