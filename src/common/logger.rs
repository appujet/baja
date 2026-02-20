use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt;
use tracing_subscriber::prelude::*;

use crate::configs::Config;

pub fn init(config: &Config) {
    // Determine the base log level
    let log_level = config
        .logging
        .as_ref()
        .and_then(|l| l.level.as_deref())
        .unwrap_or("info");

    // Get any additional filters
    let filters = config
        .logging
        .as_ref()
        .and_then(|l| l.filters.as_deref())
        .unwrap_or("");

    // Construct the filter string
    let filter_str = if filters.is_empty() {
        log_level.to_string()
    } else {
        format!("{},{}", log_level, filters)
    };

    // Create the environment filter, allowing RUST_LOG to override
    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(filter_str));

    // Base console layer
    let stdout_layer = fmt::layer()
        .with_target(true)
        .with_thread_ids(true)
        .with_line_number(true)
        .with_file(false);

    // Optional file layer
    let file_layer = if let Some(logging) = &config.logging {
        if let Some(file_config) = &logging.file {
            // Ensure directory exists
            if let Some(parent) = Path::new(&file_config.path).parent() {
                if let Err(e) = fs::create_dir_all(parent) {
                    eprintln!("Failed to create log directory: {}", e);
                }
            }

            let writer = CircularFileWriter::new(file_config.path.clone(), file_config.max_lines);
            Some(
                fmt::layer()
                    .with_writer(writer)
                    .with_target(true)
                    .with_thread_ids(true)
                    .with_line_number(true)
                    .with_file(false)
                    .with_ansi(false), // Files shouldn't usually have ANSI codes
            )
        } else {
            None
        }
    } else {
        None
    };

    // Initialize the subscriber with both layers
    tracing_subscriber::registry()
        .with(env_filter)
        .with(stdout_layer)
        .with(file_layer)
        .init();
}

/// A simple writer that appends to a file and periodically prunes old lines
/// to stay under a maximum line count.
#[derive(Clone)]
struct CircularFileWriter {
    path: String,
    max_lines: u32,
    state: Arc<Mutex<WriterState>>,
}

struct WriterState {
    lines_since_prune: u32,
}

impl CircularFileWriter {
    fn new(path: String, max_lines: u32) -> Self {
        Self {
            path,
            max_lines,
            state: Arc::new(Mutex::new(WriterState {
                lines_since_prune: 0,
            })),
        }
    }

    fn prune(&self) -> io::Result<()> {
        if !Path::new(&self.path).exists() {
            return Ok(());
        }

        let file = File::open(&self.path)?;
        let reader = BufReader::new(file);
        let lines: Vec<String> = reader.lines().collect::<Result<_, _>>()?;

        if lines.len() > self.max_lines as usize {
            let start = lines.len() - self.max_lines as usize;
            let mut file = File::create(&self.path)?;
            for line in &lines[start..] {
                writeln!(file, "{}", line)?;
            }
        }
        Ok(())
    }
}

impl io::Write for CircularFileWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;

        file.write_all(buf)?;

        let mut state = self.state.lock().unwrap();
        let new_lines = buf.iter().filter(|&&b| b == b'\n').count() as u32;
        state.lines_since_prune += new_lines;

        // Prune if we've added enough lines
        // We prune when we added 10% of max_lines or at least 50 lines.
        let prune_threshold = (self.max_lines / 10).max(50);
        if state.lines_since_prune >= prune_threshold {
            if let Err(e) = self.prune() {
                eprintln!("Failed to prune log file: {}", e);
            }
            state.lines_since_prune = 0;
        }

        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> fmt::MakeWriter<'a> for CircularFileWriter {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}
