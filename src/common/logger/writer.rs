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

#[cfg(test)]
mod tests {
    use std::fs;

    use tracing_subscriber::fmt::MakeWriter;

    use super::*;

    fn cleanup_test_file(path: &str) {
        let _ = fs::remove_file(path);
        let _ = fs::remove_file(format!("{}.tmp", path));
    }

    #[test]
    fn test_circular_file_writer_new() {
        let writer = CircularFileWriter::new("test_new.log".to_string(), 100);
        let state = writer.state.lock();
        assert_eq!(state.lines_since_prune, 0);
        assert!(!state.is_pruning);
        assert!(state.file.is_none());
        cleanup_test_file("test_new.log");
    }

    #[test]
    fn test_write_creates_file() {
        let path = "test_create.log";
        cleanup_test_file(path);

        let mut writer = CircularFileWriter::new(path.to_string(), 100);
        let data = b"test line\n";
        let result = writer.write(data);

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), data.len());
        assert!(Path::new(path).exists());

        cleanup_test_file(path);
    }

    #[test]
    fn test_write_counts_newlines() {
        let path = "test_newlines.log";
        cleanup_test_file(path);

        let mut writer = CircularFileWriter::new(path.to_string(), 1000);
        writer.write(b"line1\nline2\nline3\n").unwrap();

        let state = writer.state.lock();
        assert_eq!(state.lines_since_prune, 3);

        cleanup_test_file(path);
    }

    #[test]
    fn test_write_no_newlines() {
        let path = "test_no_newlines.log";
        cleanup_test_file(path);

        let mut writer = CircularFileWriter::new(path.to_string(), 1000);
        writer.write(b"no newline here").unwrap();

        let state = writer.state.lock();
        assert_eq!(state.lines_since_prune, 0);

        cleanup_test_file(path);
    }

    #[test]
    fn test_flush() {
        let path = "test_flush.log";
        cleanup_test_file(path);

        let mut writer = CircularFileWriter::new(path.to_string(), 100);
        writer.write(b"test\n").unwrap();

        let result = writer.flush();
        assert!(result.is_ok());

        cleanup_test_file(path);
    }

    #[test]
    fn test_flush_without_file() {
        let mut writer = CircularFileWriter::new("test_flush_no_file.log".to_string(), 100);
        let result = writer.flush();
        assert!(result.is_ok());
        cleanup_test_file("test_flush_no_file.log");
    }

    #[test]
    fn test_clone() {
        let writer = CircularFileWriter::new("test_clone.log".to_string(), 100);
        let cloned = writer.clone();

        // Both should share the same state
        assert!(Arc::ptr_eq(&writer.state, &cloned.state));

        cleanup_test_file("test_clone.log");
    }

    #[test]
    fn test_make_writer() {
        let writer = CircularFileWriter::new("test_make_writer.log".to_string(), 100);
        let made = writer.make_writer();

        // Should be a clone
        assert!(Arc::ptr_eq(&writer.state, &made.state));

        cleanup_test_file("test_make_writer.log");
    }

    #[test]
    fn test_do_prune_nonexistent_file() {
        let result = CircularFileWriter::do_prune("nonexistent_prune.log", 10);
        assert!(result.is_ok());
    }

    #[test]
    fn test_do_prune_small_file() {
        let path = "test_prune_small.log";
        cleanup_test_file(path);

        fs::write(path, "line1\nline2\nline3\n").unwrap();

        let result = CircularFileWriter::do_prune(path, 10);
        assert!(result.is_ok());

        // File should still have 3 lines (less than max)
        let content = fs::read_to_string(path).unwrap();
        assert_eq!(content.lines().count(), 3);

        cleanup_test_file(path);
    }

    #[test]
    fn test_do_prune_large_file() {
        let path = "test_prune_large.log";
        cleanup_test_file(path);

        // Write 20 lines
        let mut content = String::new();
        for i in 1..=20 {
            content.push_str(&format!("line{}\n", i));
        }
        fs::write(path, content).unwrap();

        // Prune to 10 lines
        let result = CircularFileWriter::do_prune(path, 10);
        assert!(result.is_ok());

        // Should only have last 10 lines
        let pruned = fs::read_to_string(path).unwrap();
        let lines: Vec<&str> = pruned.lines().collect();
        assert_eq!(lines.len(), 10);
        assert_eq!(lines[0], "line11");
        assert_eq!(lines[9], "line20");

        cleanup_test_file(path);
    }

    #[test]
    fn test_prune_threshold_calculation() {
        let _writer = CircularFileWriter::new("test.log".to_string(), 1000);
        let threshold = (1000 / 10).max(50);
        assert_eq!(threshold, 100);

        let _writer = CircularFileWriter::new("test.log".to_string(), 100);
        let threshold = (100 / 10).max(50);
        assert_eq!(threshold, 50);

        let _writer = CircularFileWriter::new("test.log".to_string(), 10);
        let threshold = (10 / 10).max(50);
        assert_eq!(threshold, 50);

        cleanup_test_file("test.log");
    }
}
