//! `SegmentedSource` — parallel-chunk HTTP audio source.
//! Workers pre-fetch a sliding window of chunks ahead of the read cursor.
//! Seeking is instant — just update `current_pos`; workers naturally
//! re-prioritize chunks around the new position.

use std::{
    collections::HashMap,
    io::{Read, Seek, SeekFrom},
    sync::Arc,
    thread,
    time::Duration,
};

use parking_lot::{Condvar, Mutex};
use symphonia::core::io::MediaSource;
use tracing::{debug, trace, warn};

use crate::{
    audio::constants::{
        CHUNK_SIZE, FETCH_WAIT_MS, MAX_CONCURRENT_FETCHES, MAX_FETCH_RETRIES, PREFETCH_CHUNKS,
        PROBE_TIMEOUT_SECS, WORKER_IDLE_MS,
    },
    common::types::AnyResult,
};

use super::AudioSource;

// ─────────────────────────────────────────────────────────────────────────────
// Internal types
// ─────────────────────────────────────────────────────────────────────────────

/// State of a single downloaded chunk.
#[derive(Clone)]
enum ChunkState {
    /// Not yet scheduled; inner value is the number of previous failed attempts.
    Empty(usize),
    /// A worker has claimed this chunk and is downloading it.
    Downloading,
    /// Data is available for reading.
    Ready(Arc<Vec<u8>>),
}

struct ReaderState {
    chunks: HashMap<usize, ChunkState>,
    current_pos: u64,
    total_len: u64,
    is_terminated: bool,
    fatal_error: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// SegmentedSource
// ─────────────────────────────────────────────────────────────────────────────

/// Parallel-chunk HTTP source for large, seekable streams (e.g. YouTube audio).
pub struct SegmentedSource {
    pos: u64,
    len: u64,
    /// Stored as `Arc<str>` so `content_type()` clones are pointer-bump cheap.
    content_type: Option<Arc<str>>,
    shared: Arc<(Mutex<ReaderState>, Condvar)>,
}

impl SegmentedSource {
    /// Open `url` and start the background fetch workers.
    ///
    /// Performs a Range probe on `bytes=0-0` to determine content length.
    pub fn new(client: reqwest::Client, url: &str) -> AnyResult<Self> {
        let handle = tokio::runtime::Handle::current();

        let probe = handle.block_on(
            client
                .get(url)
                .header("Range", "bytes=0-0")
                .header("Connection", "close")
                .timeout(Duration::from_secs(PROBE_TIMEOUT_SECS))
                .send(),
        )?;

        let len = probe
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split('/').last())
            .and_then(|v| v.parse::<u64>().ok())
            .or_else(|| probe.content_length())
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "SegmentedSource: could not determine content length",
                )
            })?;

        let content_type: Option<Arc<str>> = probe
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(Arc::from);

        debug!("SegmentedSource opened: len={}, type={:?}", len, content_type);

        let mut chunks = HashMap::new();
        chunks.insert(0, ChunkState::Empty(0));

        let shared = Arc::new((
            Mutex::new(ReaderState {
                chunks,
                current_pos: 0,
                total_len: len,
                is_terminated: false,
                fatal_error: None,
            }),
            Condvar::new(),
        ));

        for worker_id in 0..MAX_CONCURRENT_FETCHES {
            let shared_clone = shared.clone();
            let client_clone = client.clone();
            let url_str = url.to_string();
            let handle_clone = handle.clone();
            thread::Builder::new()
                .name(format!("segmented-fetch-{}", worker_id))
                .spawn(move || {
                    fetch_worker(worker_id, shared_clone, client_clone, url_str, handle_clone);
                })?;
        }

        Ok(Self {
            pos: 0,
            len,
            content_type,
            shared,
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Trait impls
// ─────────────────────────────────────────────────────────────────────────────

impl AudioSource for SegmentedSource {
    fn content_type(&self) -> Option<String> {
        // Arc<str> clone is a pointer-bump — no string copy.
        self.content_type.as_deref().map(str::to_string)
    }
}

impl Read for SegmentedSource {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.pos >= self.len {
            return Ok(0);
        }

        let (lock, cvar) = &*self.shared;
        let mut state = lock.lock();
        state.current_pos = self.pos;

        loop {
            if let Some(ref err) = state.fatal_error {
                return Err(std::io::Error::new(std::io::ErrorKind::Other, err.clone()));
            }

            let chunk_idx = (self.pos / CHUNK_SIZE as u64) as usize;
            let offset_in_chunk = (self.pos % CHUNK_SIZE as u64) as usize;

            match state.chunks.get(&chunk_idx) {
                Some(ChunkState::Ready(data)) => {
                    let data = Arc::clone(data);
                    let available = data.len().saturating_sub(offset_in_chunk);

                    if available == 0 {
                        // Chunk is exhausted; advance to the next one.
                        self.pos = ((chunk_idx + 1) * CHUNK_SIZE) as u64;
                        state.current_pos = self.pos;
                        continue;
                    }

                    let n = buf.len().min(available);
                    buf[..n].copy_from_slice(&data[offset_in_chunk..offset_in_chunk + n]);
                    self.pos += n as u64;
                    state.current_pos = self.pos;

                    // Evict chunks that are no longer needed, keeping one behind the
                    // cursor as a small backward-seek buffer.
                    if chunk_idx > 1 {
                        state.chunks.retain(|&idx, _| idx >= chunk_idx - 1);
                    }

                    return Ok(n);
                }

                Some(ChunkState::Downloading) | Some(ChunkState::Empty(_)) => {
                    cvar.notify_all();
                    trace!("SegmentedSource: waiting for chunk {}", chunk_idx);
                    cvar.wait_for(&mut state, Duration::from_millis(FETCH_WAIT_MS));
                }

                None => {
                    state.chunks.insert(chunk_idx, ChunkState::Empty(0));
                    cvar.notify_all();
                }
            }
        }
    }
}

impl Seek for SegmentedSource {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        let new_pos = match pos {
            SeekFrom::Start(p) => p,
            SeekFrom::Current(delta) => self.pos.saturating_add_signed(delta),
            SeekFrom::End(delta) => self.len.saturating_add_signed(delta),
        };

        self.pos = new_pos.min(self.len);
        debug!("SegmentedSource: seek → {}", self.pos);

        let (lock, cvar) = &*self.shared;
        let mut state = lock.lock();
        state.current_pos = self.pos;
        cvar.notify_all();

        Ok(self.pos)
    }
}

impl MediaSource for SegmentedSource {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        Some(self.len)
    }
}

impl Drop for SegmentedSource {
    fn drop(&mut self) {
        let (lock, cvar) = &*self.shared;
        let mut state = lock.lock();
        state.is_terminated = true;
        cvar.notify_all();
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Issue a Range request for `[offset, offset + size)` and return the response.
async fn fetch_range(
    client: &reqwest::Client,
    url: &str,
    offset: u64,
    size: u64,
) -> AnyResult<reqwest::Response> {
    let range = format!("bytes={}-{}", offset, offset + size - 1);
    let res = client
        .get(url)
        .header("Range", range)
        .header("Accept", "*/*")
        .send()
        .await?;

    // 206 Partial Content is what we expect; any non-2xx is an error.
    if !res.status().is_success() {
        return Err(format!("fetch_range: HTTP {}", res.status()).into());
    }
    Ok(res)
}

// ─────────────────────────────────────────────────────────────────────────────
// Fetch worker
// ─────────────────────────────────────────────────────────────────────────────

fn fetch_worker(
    worker_id: usize,
    shared: Arc<(Mutex<ReaderState>, Condvar)>,
    client: reqwest::Client,
    url: String,
    handle: tokio::runtime::Handle,
) {
    let (lock, cvar) = &*shared;

    loop {
        // ── Claim a chunk to download ─────────────────────────────────────────
        let target = {
            let mut state = lock.lock();

            if state.is_terminated {
                break;
            }

            let current_chunk = (state.current_pos / CHUNK_SIZE as u64) as usize;
            let total_len = state.total_len;

            // Try to claim the cursor chunk first; if it is already handled,
            // look ahead in the prefetch window.
            let claimed = try_claim_chunk(&mut state, current_chunk, total_len);

            if claimed.is_none() {
                let cursor_ready = matches!(
                    state.chunks.get(&current_chunk),
                    Some(ChunkState::Ready(_))
                );
                let window = if cursor_ready { PREFETCH_CHUNKS } else { 2 };

                let mut found = None;
                for j in 1..window {
                    let idx = current_chunk + j;
                    if (idx * CHUNK_SIZE) as u64 >= total_len {
                        break;
                    }
                    if let Some(c) = try_claim_chunk(&mut state, idx, total_len) {
                        found = Some(c);
                        break;
                    }
                }
                found
            } else {
                claimed
            }
            .map(|(idx, retries)| {
                // Mark as in-flight *before* releasing the lock.
                state.chunks.insert(idx, ChunkState::Downloading);
                (idx, retries, total_len)
            })
        };

        let (idx, prior_retries, total_len) = match target {
            Some(t) => t,
            None => {
                // Nothing to do — wait briefly.
                let mut state = lock.lock();
                cvar.wait_for(&mut state, Duration::from_millis(WORKER_IDLE_MS));
                continue;
            }
        };

        // ── Download the chunk ────────────────────────────────────────────────
        let offset = (idx * CHUNK_SIZE) as u64;
        let size = CHUNK_SIZE.min((total_len - offset) as usize) as u64;

        trace!(
            "Worker {}: requesting chunk {} (offset={}, size={})",
            worker_id, idx, offset, size
        );

        match handle.block_on(fetch_range(&client, &url, offset, size)) {
            Ok(res) => match handle.block_on(res.bytes()) {
                Ok(bytes) => {
                    let actual = bytes.len();
                    let arc = Arc::new(bytes.to_vec());
                    let mut state = lock.lock();
                    state.chunks.insert(idx, ChunkState::Ready(arc));
                    trace!("Worker {}: filled chunk {} ({} bytes)", worker_id, idx, actual);
                    cvar.notify_all();
                }
                Err(e) => {
                    warn!("Worker {}: failed to read body for chunk {}: {}", worker_id, idx, e);
                    requeue_or_fatal(lock, cvar, idx, prior_retries, &e.to_string());
                    thread::sleep(Duration::from_millis(FETCH_WAIT_MS));
                }
            },
            Err(e) => {
                warn!("Worker {}: fetch failed for chunk {}: {}", worker_id, idx, e);
                requeue_or_fatal(lock, cvar, idx, prior_retries, &e.to_string());
                thread::sleep(Duration::from_millis(FETCH_WAIT_MS));
            }
        }
    }
}

/// Try to claim a chunk at `idx` if it is in `Empty` state.
///
/// Returns `Some((idx, retry_count))` on success, `None` if the chunk is
/// already `Downloading` or `Ready`.
#[inline]
fn try_claim_chunk(
    state: &mut ReaderState,
    idx: usize,
    total_len: u64,
) -> Option<(usize, usize)> {
    if (idx * CHUNK_SIZE) as u64 >= total_len {
        return None;
    }
    match state.chunks.get(&idx) {
        Some(ChunkState::Empty(r)) => Some((idx, *r)),
        None => {
            // Newly seen chunk — treat as Empty(0).
            Some((idx, 0))
        }
        _ => None,
    }
}

/// On a download failure, either requeue the chunk for retry or mark the
/// source as fatally errored if `MAX_FETCH_RETRIES` is exceeded.
#[inline]
fn requeue_or_fatal(
    lock: &Mutex<ReaderState>,
    cvar: &Condvar,
    idx: usize,
    prior_retries: usize,
    error: &str,
) {
    let mut state = lock.lock();
    if prior_retries >= MAX_FETCH_RETRIES {
        state.fatal_error = Some(format!(
            "Chunk {}: permanently failed after {} retries: {}",
            idx, prior_retries, error
        ));
    } else {
        state.chunks.insert(idx, ChunkState::Empty(prior_retries + 1));
    }
    cvar.notify_all();
}
