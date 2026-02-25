//! Power-of-two aligned byte buffer pool.
//!
//! Mirrors NodeLink's `BufferPool.ts`: sizes are rounded up to the next power
//! of two (minimum 1 024 bytes), pooled in per-size buckets, and evicted after
//! a configurable idle period.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Maximum total bytes held in the pool (50 MB).
const MAX_POOL_BYTES: usize = 50 * 1024 * 1024;

/// Maximum buffers per bucket.
const MAX_BUCKET_ENTRIES: usize = 8;

/// Idle period before the pool is cleared (3 minutes).
const IDLE_CLEAR_SECS: u64 = 180;

// ── Inner state ──────────────────────────────────────────────────────────────

struct PoolInner {
    buckets: HashMap<usize, Vec<Vec<u8>>>,
    total_bytes: usize,
    last_activity: Instant,
}

impl PoolInner {
    fn new() -> Self {
        Self {
            buckets: HashMap::new(),
            total_bytes: 0,
            last_activity: Instant::now(),
        }
    }

    /// Round `size` up to the next power of two, with a floor of 1 024.
    fn aligned_size(size: usize) -> usize {
        if size <= 1024 {
            return 1024;
        }
        let mut n = size.saturating_sub(1);
        n |= n >> 1;
        n |= n >> 2;
        n |= n >> 4;
        n |= n >> 8;
        n |= n >> 16;
        n |= n >> 32;
        n + 1
    }

    fn acquire(&mut self, size: usize) -> Vec<u8> {
        self.last_activity = Instant::now();
        let aligned = Self::aligned_size(size);

        if let Some(bucket) = self.buckets.get_mut(&aligned) {
            if let Some(mut buf) = bucket.pop() {
                self.total_bytes -= aligned;
                buf.clear();
                return buf;
            }
        }
        Vec::with_capacity(aligned)
    }

    fn release(&mut self, mut buf: Vec<u8>) {
        self.last_activity = Instant::now();
        let size = buf.capacity();

        // Only pool buffers in the 1 KB – 10 MB range.
        if size < 1024 || size > 10 * 1024 * 1024 {
            return;
        }
        if self.total_bytes + size > MAX_POOL_BYTES {
            return;
        }

        let bucket = self.buckets.entry(size).or_default();
        if bucket.len() >= MAX_BUCKET_ENTRIES {
            return; // bucket full — just drop
        }

        buf.clear();
        self.total_bytes += size;
        bucket.push(buf);
    }

    /// Evict all buffers if the pool has been idle for `IDLE_CLEAR_SECS`.
    fn maybe_cleanup(&mut self) {
        if self.total_bytes == 0 {
            return;
        }
        if self.last_activity.elapsed() >= Duration::from_secs(IDLE_CLEAR_SECS) {
            self.buckets.clear();
            self.total_bytes = 0;
        } else if self.total_bytes > MAX_POOL_BYTES {
            self.buckets.clear();
            self.total_bytes = 0;
        }
    }
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Shared, thread-safe byte buffer pool.
pub struct BufferPool {
    inner: Mutex<PoolInner>,
}

impl BufferPool {
    fn new() -> Self {
        Self {
            inner: Mutex::new(PoolInner::new()),
        }
    }

    /// Acquire a buffer of at least `size` bytes.
    pub fn acquire(&self, size: usize) -> Vec<u8> {
        let mut g = self.inner.lock().unwrap();
        g.maybe_cleanup();
        g.acquire(size)
    }

    /// Return a buffer to the pool for reuse.
    pub fn release(&self, buf: Vec<u8>) {
        let mut g = self.inner.lock().unwrap();
        g.release(buf);
    }

    /// Pool statistics (total bytes, bucket count).
    pub fn stats(&self) -> PoolStats {
        let g = self.inner.lock().unwrap();
        PoolStats {
            total_bytes: g.total_bytes,
            buckets: g.buckets.len(),
            entries: g.buckets.values().map(|b| b.len()).sum(),
        }
    }
}

/// Snapshot of pool health.
#[derive(Debug, Clone)]
pub struct PoolStats {
    pub total_bytes: usize,
    pub buckets: usize,
    pub entries: usize,
}

// ── Global singleton ─────────────────────────────────────────────────────────

static GLOBAL_BYTE_POOL: OnceLock<Arc<BufferPool>> = OnceLock::new();

/// Get (or lazily create) the global byte buffer pool.
pub fn get_byte_pool() -> Arc<BufferPool> {
    GLOBAL_BYTE_POOL
        .get_or_init(|| Arc::new(BufferPool::new()))
        .clone()
}
