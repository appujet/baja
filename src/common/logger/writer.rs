use std::{
    fs::{File, OpenOptions},
    io::{self, BufRead, BufReader, Write},
    path::Path,
    sync::{Arc, Mutex},
};

// Simple ANSI stripper to prevent the log file from being polluted with escape sequences
pub fn strip_ansi_escapes(s: &str) -> String {
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
    pub fn new(path: String, max_lines: u32) -> Self {
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

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CircularFileWriter {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}
