use std::{
    fs::{File, OpenOptions},
    io::{self, BufRead, BufReader, Write},
    path::Path,
    sync::Arc,
};

use parking_lot::Mutex;

/// A simple writer that appends to a file and periodically prunes old lines
/// to stay under a maximum line count.
#[derive(Clone)]
pub struct CircularFileWriter {
    path: String,
    max_lines: u32,
    state: Arc<Mutex<WriterState>>,
}

struct WriterState {
    file: Option<File>,
    lines_since_prune: u32,
    is_pruning: bool,
}

impl CircularFileWriter {
    pub fn new(path: String, max_lines: u32) -> Self {
        Self {
            path,
            max_lines,
            state: Arc::new(Mutex::new(WriterState {
                file: None,
                lines_since_prune: 0,
                is_pruning: false,
            })),
        }
    }

    fn ensure_file_open<'a>(&self, state: &'a mut WriterState) -> io::Result<&'a mut File> {
        if state.file.is_none() {
            state.file = Some(
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&self.path)?,
            );
        }
        Ok(state.file.as_mut().expect("file was just opened"))
    }

    fn spawn_prune(&self) {
        let path = self.path.clone();
        let max_lines = self.max_lines;
        let state_arc = self.state.clone();

        std::thread::spawn(move || {
            if let Err(e) = Self::do_prune(&path, max_lines) {
                eprintln!("Failed to prune log file '{}': {}", path, e);
            }
            let mut state = state_arc.lock();
            state.is_pruning = false;
        });
    }

    fn do_prune(path: &str, max_lines: u32) -> io::Result<()> {
        if !Path::new(path).exists() {
            return Ok(());
        }

        let lines: Vec<String> = {
            let file = File::open(path)?;
            let reader = BufReader::new(file);
            reader.lines().collect::<Result<_, _>>()?
        };

        if lines.len() > max_lines as usize {
            let start = lines.len() - max_lines as usize;
            // Atomic-ish replacement: write to .tmp then rename
            let tmp_path = format!("{}.tmp", path);
            {
                let mut file = File::create(&tmp_path)?;
                for line in &lines[start..] {
                    writeln!(file, "{}", line)?;
                }
            }
            std::fs::rename(tmp_path, path)?;
        }
        Ok(())
    }
}

impl io::Write for CircularFileWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut state = self.state.lock();
        
        let file = self.ensure_file_open(&mut state)?;
        file.write_all(buf)?;

        let new_lines = buf.iter().filter(|&&b| b == b'\n').count() as u32;
        state.lines_since_prune += new_lines;

        let prune_threshold = (self.max_lines / 10).max(50);
        if state.lines_since_prune >= prune_threshold && !state.is_pruning {
            state.is_pruning = true;
            state.lines_since_prune = 0;
            state.file = None; // Close file so rename can happen on Windows if needed
            self.spawn_prune();
        }

        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        let mut state = self.state.lock();
        if let Some(file) = &mut state.file {
            file.flush()?;
        }
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CircularFileWriter {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}
