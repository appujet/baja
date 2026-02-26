//! Fixed-size circular byte buffer.
//!
//! Mirrors NodeLink's `RingBuffer.ts`: a power-of-two-sized ring that supports
//! wrap-around writes and reads. Backed by the global [`BufferPool`] to avoid
//! heap allocations in the hot path.

use crate::audio::buffer::pool::get_byte_pool;

pub struct RingBuffer {
    buf: Vec<u8>,
    size: usize,
    write_offset: usize,
    read_offset: usize,
    length: usize,
}

impl RingBuffer {
    /// Create a new `RingBuffer` of `size` bytes.
    pub fn new(size: usize) -> Self {
        let pool = get_byte_pool();
        let mut buf = pool.acquire(size);
        buf.resize(size, 0);
        Self {
            buf,
            size,
            write_offset: 0,
            read_offset: 0,
            length: 0,
        }
    }

    /// How many bytes are currently available to read.
    pub fn len(&self) -> usize {
        self.length
    }

    pub fn is_empty(&self) -> bool {
        self.length == 0
    }

    /// How many bytes can still be written before the buffer is full.
    pub fn remaining(&self) -> usize {
        self.size - self.length
    }

    /// Write `chunk` into the buffer.  
    /// If the buffer is full, the **oldest** data is overwritten.
    pub fn write(&mut self, chunk: &[u8]) {
        let to_write = chunk.len();
        let available_at_end = self.size - self.write_offset;

        if to_write <= available_at_end {
            self.buf[self.write_offset..self.write_offset + to_write].copy_from_slice(chunk);
        } else {
            // Wrap around
            self.buf[self.write_offset..].copy_from_slice(&chunk[..available_at_end]);
            self.buf[..to_write - available_at_end].copy_from_slice(&chunk[available_at_end..]);
        }

        let new_len = self.length + to_write;
        if new_len > self.size {
            // Overwrite oldest — advance read pointer
            let overwritten = new_len - self.size;
            self.read_offset = (self.read_offset + overwritten) % self.size;
            self.length = self.size;
        } else {
            self.length = new_len;
        }

        self.write_offset = (self.write_offset + to_write) % self.size;
    }

    /// Read up to `n` bytes, returning them in a pooled `Vec<u8>`.
    /// Returns `None` if the buffer is empty.
    pub fn read(&mut self, n: usize) -> Option<Vec<u8>> {
        let to_read = n.min(self.length);
        if to_read == 0 {
            return None;
        }

        let pool = get_byte_pool();
        let mut out = pool.acquire(to_read);
        out.resize(to_read, 0);

        let available_at_end = self.size - self.read_offset;
        if to_read <= available_at_end {
            out[..to_read].copy_from_slice(&self.buf[self.read_offset..self.read_offset + to_read]);
        } else {
            out[..available_at_end].copy_from_slice(&self.buf[self.read_offset..]);
            out[available_at_end..].copy_from_slice(&self.buf[..to_read - available_at_end]);
        }

        self.read_offset = (self.read_offset + to_read) % self.size;
        self.length -= to_read;
        Some(out)
    }

    /// Peek at up to `n` bytes without advancing the read pointer.
    /// Returns `None` if the buffer is empty.
    pub fn peek(&self, n: usize) -> Option<Vec<u8>> {
        let to_read = n.min(self.length);
        if to_read == 0 {
            return None;
        }

        let pool = get_byte_pool();
        let mut out = pool.acquire(to_read);
        out.resize(to_read, 0);

        let available_at_end = self.size - self.read_offset;
        if to_read <= available_at_end {
            out[..to_read].copy_from_slice(&self.buf[self.read_offset..self.read_offset + to_read]);
        } else {
            out[..available_at_end].copy_from_slice(&self.buf[self.read_offset..]);
            out[available_at_end..].copy_from_slice(&self.buf[..to_read - available_at_end]);
        }

        Some(out)
    }

    /// Skip `n` bytes without copying.  Returns actual bytes skipped.
    pub fn skip(&mut self, n: usize) -> usize {
        let to_skip = n.min(self.length);
        self.read_offset = (self.read_offset + to_skip) % self.size;
        self.length -= to_skip;
        to_skip
    }

    /// Reset the buffer to empty.
    pub fn clear(&mut self) {
        self.write_offset = 0;
        self.read_offset = 0;
        self.length = 0;
    }

    /// Return the internal buffer to the pool.
    pub fn dispose(mut self) {
        let pool = get_byte_pool();
        // Swap out the Vec so we can hand it back.
        let buf = std::mem::take(&mut self.buf);
        pool.release(buf);
    }
}

impl Drop for RingBuffer {
    fn drop(&mut self) {
        if !self.buf.is_empty() {
            let pool = get_byte_pool();
            let buf = std::mem::take(&mut self.buf);
            pool.release(buf);
        }
    }
}
