//! `SegmentedSource` — parallel-chunk HTTP audio source.
//!
//! Replaces `audio::remote_reader::segmented::SegmentedRemoteReader`.
//!
//! # How it works
//!
//! ```text
//!  ┌────────────────────┐  ┌────────────────────┐
//!  │  fetch-worker-0    │  │  fetch-worker-1    │  … (N workers)
//!  │  (Range requests)  │  │  (Range requests)  │
//!  └─────────┬──────────┘  └─────────┬──────────┘
//!            │ chunk map (Arc<Vec<u8>>)
//!  ┌─────────▼──────────────────────▼──────────┐
//!  │         SegmentedSource                    │ ◄── Read / Seek
//!  └────────────────────────────────────────────┘
//! ```
//!
//! Workers pre-fetch a sliding window of chunks ahead of the read cursor.
//! Seeking is instant — just update `current_pos`; workers naturally
//! re-prioritize chunks around the new position.

use std::{
    collections::HashMap,
    io::{Read, Seek, SeekFrom},
    sync::Arc,
    thread,
};

use parking_lot::{Condvar, Mutex};
use symphonia::core::io::MediaSource;
use tracing::{debug, trace, warn};

use crate::{
    audio::constants::{CHUNK_SIZE, MAX_CONCURRENT_FETCHES, PREFETCH_CHUNKS},
    common::types::AnyResult,
};

use super::AudioSource;

// ─── Internal chunk state ─────────────────────────────────────────────────────

#[derive(Clone)]
enum ChunkState {
    Empty(usize), // retry count
    Downloading,
    Ready(Arc<Vec<u8>>),
}

struct ReaderState {
    chunks: HashMap<usize, ChunkState>,
    current_pos: u64,
    total_len: u64,
    is_terminated: bool,
    fatal_error: Option<String>,
}

// ─── SegmentedSource ──────────────────────────────────────────────────────────

/// Parallel-chunk HTTP source — renamed from `SegmentedRemoteReader`.
///
/// Best for large, seekable streams (e.g. YouTube audio).
pub struct SegmentedSource {
    pos: u64,
    len: u64,
    content_type: Option<String>,
    shared: Arc<(Mutex<ReaderState>, Condvar)>,
}

impl SegmentedSource {
    /// Open `url` and start the background fetch workers.
    ///
    /// Performs a HEAD-like Range probe to determine content length.
    pub fn new(client: reqwest::Client, url: &str) -> AnyResult<Self> {
        let handle = tokio::runtime::Handle::current();

        let probe = handle.block_on(
            client
                .get(url)
                .header("Range", "bytes=0-0")
                .header("Connection", "close")
                .timeout(std::time::Duration::from_secs(10))
                .send(),
        )?;

        let len = probe
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split('/').last())
            .and_then(|v| v.parse::<u64>().ok())
            .or_else(|| probe.content_length())
            .ok_or("SegmentedSource: could not determine content length")?;

        let content_type = probe
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);

        debug!(
            "Opened SegmentedSource: (len={}, type={:?})",
            len, content_type
        );

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

        for i in 0..MAX_CONCURRENT_FETCHES {
            let shared_clone = shared.clone();
            let client_clone = client.clone();
            let url_str = url.to_string();
            let handle_clone = handle.clone();
            thread::Builder::new()
                .name(format!("segmented-fetch-{}", i))
                .spawn(move || {
                    fetch_worker(i, shared_clone, client_clone, url_str, handle_clone);
                })?;
        }

        Ok(Self {
            pos: 0,
            len,
            content_type,
            shared,
        })
    }

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

        if !res.status().is_success() && res.status() != reqwest::StatusCode::PARTIAL_CONTENT {
            return Err(format!("Fetch failed: status={}", res.status()).into());
        }
        Ok(res)
    }
}

// ─── AudioSource ─────────────────────────────────────────────────────────────

impl AudioSource for SegmentedSource {
    fn content_type(&self) -> Option<String> {
        self.content_type.clone()
    }
}

// ─── Read / Seek / MediaSource ────────────────────────────────────────────────

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
                    let available = data.len().saturating_sub(offset_in_chunk);
                    if available == 0 {
                        if self.pos >= self.len {
                            return Ok(0);
                        }
                        self.pos = ((chunk_idx + 1) * CHUNK_SIZE) as u64;
                        state.current_pos = self.pos;
                        continue;
                    }

                    let n = buf.len().min(available);
                    buf[..n].copy_from_slice(&data[offset_in_chunk..offset_in_chunk + n]);
                    self.pos += n as u64;
                    state.current_pos = self.pos;

                    // Evict old chunks to bound memory usage.
                    if chunk_idx > 1 {
                        state.chunks.retain(|&idx, _| idx >= chunk_idx - 1);
                    }
                    return Ok(n);
                }
                Some(ChunkState::Downloading) | Some(ChunkState::Empty(_)) => {
                    cvar.notify_all();
                    trace!("SegmentedSource: waiting for chunk {}", chunk_idx);
                    cvar.wait_for(&mut state, std::time::Duration::from_millis(500));
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

// ─── Fetch workers ────────────────────────────────────────────────────────────

fn fetch_worker(
    i: usize,
    shared: Arc<(Mutex<ReaderState>, Condvar)>,
    client: reqwest::Client,
    url: String,
    handle: tokio::runtime::Handle,
) {
    let (lock, cvar) = &*shared;

    loop {
        let mut target_chunk_idx = None;

        {
            let mut state = lock.lock();
            if state.is_terminated {
                break;
            }

            let current_chunk_idx = (state.current_pos / CHUNK_SIZE as u64) as usize;

            let entry = state
                .chunks
                .entry(current_chunk_idx)
                .or_insert(ChunkState::Empty(0));
            if matches!(entry, ChunkState::Empty(_)) {
                *entry = ChunkState::Downloading;
                target_chunk_idx = Some(current_chunk_idx);
            } else {
                let cursor_ready = matches!(
                    state.chunks.get(&current_chunk_idx),
                    Some(ChunkState::Ready(_))
                );
                let window_limit = if cursor_ready { PREFETCH_CHUNKS } else { 2 };

                for j in 1..window_limit {
                    let idx = current_chunk_idx + j;
                    if (idx * CHUNK_SIZE) as u64 >= state.total_len {
                        break;
                    }

                    let entry = state.chunks.entry(idx).or_insert(ChunkState::Empty(0));
                    if matches!(entry, ChunkState::Empty(_)) {
                        *entry = ChunkState::Downloading;
                        target_chunk_idx = Some(idx);
                        break;
                    }
                }
            }

            if target_chunk_idx.is_none() {
                cvar.wait_for(&mut state, std::time::Duration::from_millis(50));
                continue;
            }
        }

        if let Some(idx) = target_chunk_idx {
            let offset = (idx * CHUNK_SIZE) as u64;
            let stream_len = lock.lock().total_len;
            let size = CHUNK_SIZE.min((stream_len - offset) as usize);

            trace!("Worker {}: requesting chunk {} (offset={})", i, idx, offset);

            let fetch_fut = SegmentedSource::fetch_range(&client, &url, offset, size as u64);
            match handle.block_on(fetch_fut) {
                Ok(res) => {
                    if let Ok(data) = handle.block_on(res.bytes()) {
                        let data = data.to_vec();
                        let actual_len = data.len();
                        let mut state = lock.lock();
                        state.chunks.insert(idx, ChunkState::Ready(Arc::new(data)));
                        trace!("Worker {}: filled chunk {} ({} bytes)", i, idx, actual_len);
                        cvar.notify_all();
                    } else {
                        warn!("Worker {}: failed to read body for chunk {}", i, idx);
                        let mut state = lock.lock();
                        state.chunks.insert(idx, ChunkState::Empty(0));
                        cvar.notify_all();
                    }
                }
                Err(e) => {
                    warn!("Worker {}: fetch failed for chunk {}: {}", i, idx, e);
                    let mut state = lock.lock();
                    let retries = if let Some(ChunkState::Empty(r)) = state.chunks.get(&idx) {
                        *r
                    } else {
                        0
                    };
                    if retries > 5 {
                        state.fatal_error = Some(format!("Failed to fetch chunk {}: {}", idx, e));
                    } else {
                        state.chunks.insert(idx, ChunkState::Empty(retries + 1));
                    }
                    cvar.notify_all();
                    thread::sleep(std::time::Duration::from_millis(500));
                }
            }
        }
    }
}
