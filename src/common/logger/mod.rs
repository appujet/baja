use std::{fs, path::Path, sync::OnceLock};

use tracing_subscriber::{EnvFilter, fmt, prelude::*};

use crate::{common::utils::strip_ansi_escapes, config::LoggingConfig};

pub mod formatter;
pub mod writer;

pub use formatter::CustomFormatter;
pub use writer::CircularFileWriter;

pub(crate) static GLOBAL_FILE_WRITER: OnceLock<CircularFileWriter> = OnceLock::new();

#[macro_export]
macro_rules! log_print {
    ($($arg:tt)*) => {{
        let msg = format!($($arg)*);
        std::print!("{}", msg);
        $crate::common::logger::append_to_file_raw(&msg);
    }};
}

#[macro_export]
macro_rules! log_println {
    () => {{
        std::println!();
        $crate::common::logger::append_to_file_raw("\n");
    }};
    ($($arg:tt)*) => {{
        let msg = format!($($arg)*);
        std::println!("{}", msg);
        $crate::common::logger::append_to_file_raw(&format!("{}\n", msg));
    }};
}

/// Appends a raw message to the global log file, stripping ANSI escapes.
pub fn append_to_file_raw(msg: &str) {
    if let Some(mut writer) = GLOBAL_FILE_WRITER.get().cloned() {
        use std::io::Write;
        let clean_msg = strip_ansi_escapes(msg);
        let _ = writer.write_all(clean_msg.as_bytes());
    }
}

/// Initializes the global logger with the provided configuration.
pub fn init(config: &LoggingConfig) {
    let log_level = config.level.as_deref().unwrap_or("info");
    let filter_str = match config.filters.as_deref() {
        Some(f) if !f.is_empty() => format!("{log_level},{f}"),
        _ => log_level.to_string(),
    };

    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(filter_str));

    let stdout_layer = fmt::layer()
        .event_format(CustomFormatter::new(true))
        .with_ansi(true);

    let file_layer = config.file.as_ref().map(|file_config| {
        if let Some(parent) = Path::new(&file_config.path).parent() {
            let _ = fs::create_dir_all(parent);
        }

        let writer = CircularFileWriter::new(file_config.path.clone(), file_config.max_lines);
        let _ = GLOBAL_FILE_WRITER.set(writer.clone());

        fmt::layer()
            .with_writer(writer)
            .event_format(CustomFormatter::new(false))
            .with_ansi(false)
    });

    tracing_subscriber::registry()
        .with(env_filter)
        .with(stdout_layer)
        .with(file_layer)
        .init();
}
