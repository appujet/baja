use std::{
  fs::{self, File, OpenOptions},
  io::{self, BufRead, BufReader, Write},
  path::Path,
  sync::{Arc, Mutex, OnceLock},
};

use tracing_subscriber::{EnvFilter, fmt::{self, time::LocalTime}, prelude::*};

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
    // Strip ANSI escape sequences loosely before writing to file
    let clean_msg = strip_ansi_escapes(msg);
    let _ = writer.write_all(clean_msg.as_bytes());
  }
}

// Simple ANSI stripper to prevent the log file from being polluted with escape sequences
fn strip_ansi_escapes(s: &str) -> String {
  let mut result = String::with_capacity(s.len());
  let mut in_escape = false;
  for c in s.chars() {
    if c == '\x1b' {
      in_escape = true;
    } else if in_escape {
      if c.is_ascii_alphabetic() {
        in_escape = false;
      }
    } else {
      result.push(c);
    }
  }
  result
}

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
  let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(filter_str));

  // Base console layer
  let stdout_layer = fmt::layer()
    .with_timer(LocalTime::rfc_3339())
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
      let _ = GLOBAL_FILE_WRITER.set(writer.clone());
      Some(
        fmt::layer()
          .with_writer(writer)
          .with_timer(LocalTime::rfc_3339())
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
pub(crate) struct CircularFileWriter {
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

    let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
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
