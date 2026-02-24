use std::{
    collections::VecDeque,
    io::{self, Read, Seek, SeekFrom},
};

use symphonia::core::io::MediaSource;
use tokio::{sync::mpsc, task::JoinHandle};
use tokio_util::bytes::Bytes;

/// Bridges SABR's async `mpsc::Receiver<Bytes>` into Symphonia's sync `Read + Seek`.
///
/// This reader is forward-only — `Seek` is not supported. `byte_len()` returns `None`
/// since SABR streams have unknown total length.
pub struct SabrReader {
    rx: mpsc::Receiver<Bytes>,
    buffer: VecDeque<u8>,
    _handle: JoinHandle<()>, // keeps the polling task alive
    finished: bool,
}

impl SabrReader {
    pub fn new(rx: mpsc::Receiver<Bytes>, handle: JoinHandle<()>) -> Self {
        Self {
            rx,
            buffer: VecDeque::new(),
            _handle: handle,
            finished: false,
        }
    }

    /// Block until we have at least `n` bytes in the buffer, or the stream ends.
    fn fill_buffer(&mut self, n: usize) {
        while self.buffer.len() < n && !self.finished {
            match self.rx.blocking_recv() {
                Some(chunk) => {
                    self.buffer.extend(chunk.iter().copied());
                }
                None => {
                    // Channel closed — stream is done
                    self.finished = true;
                    break;
                }
            }
        }
    }
}

impl Read for SabrReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }

        // Try to fill buffer if empty
        if self.buffer.is_empty() && !self.finished {
            self.fill_buffer(1);
        }

        if self.buffer.is_empty() {
            // EOF
            return Ok(0);
        }

        let n = buf.len().min(self.buffer.len());
        for (dst, src) in buf[..n].iter_mut().zip(self.buffer.drain(..n)) {
            *dst = src;
        }
        Ok(n)
    }
}

impl Seek for SabrReader {
    fn seek(&mut self, _pos: SeekFrom) -> io::Result<u64> {
        // SABR streams are forward-only
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "SABR streaming does not support seeking",
        ))
    }
}

impl MediaSource for SabrReader {
    fn is_seekable(&self) -> bool {
        false
    }

    fn byte_len(&self) -> Option<u64> {
        None
    }
}
