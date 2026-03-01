use std::{fs, path::Path, sync::OnceLock};

use tracing_subscriber::{EnvFilter, fmt, prelude::*};

pub mod formatter;
pub mod writer;

pub use formatter::*;
pub use writer::*;

use crate::configs::Config;

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

pub fn append_to_file_raw(msg: &str) {
    if let Some(mut writer) = GLOBAL_FILE_WRITER.get().cloned() {
        use std::io::Write;
        let clean_msg = strip_ansi_escapes(msg);
        let _ = writer.write_all(clean_msg.as_bytes());
    }
}

pub fn init(config: &Config) {
    let log_level = config
        .logging
        .as_ref()
        .and_then(|l| l.level.as_deref())
        .unwrap_or("info");

    let filters = config
        .logging
        .as_ref()
        .and_then(|l| l.filters.as_deref())
        .unwrap_or("");

    let filter_str = if filters.is_empty() {
        format!("{},log=error", log_level)
    } else {
        format!("{},log=error,{}", log_level, filters)
    };

    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(filter_str));

    let stdout_layer = fmt::layer()
        .event_format(CustomFormatter::new(true))
        .with_ansi(true);

    let file_layer = if let Some(logging) = &config.logging {
        if let Some(file_config) = &logging.file {
            if let Some(parent) = Path::new(&file_config.path).parent() {
                if let Err(e) = fs::create_dir_all(parent) {
                    eprintln!("Failed to create log directory: {}", e);
                }
            }

            let writer = CircularFileWriter::new(file_config.path.clone(), file_config.max_lines);
            let _ = GLOBAL_FILE_WRITER.set(writer.clone());
            Some(
                fmt::layer()
                    .with_writer(writer)
                    .event_format(CustomFormatter::new(false))
                    .with_ansi(false),
            )
        } else {
            None
        }
    } else {
        None
    };

    tracing_subscriber::registry()
        .with(env_filter)
        .with(stdout_layer)
        .with(file_layer)
        .init();
}
