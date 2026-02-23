use std::{
    collections::HashMap,
    io::{Read, Seek, SeekFrom},
    sync::{Arc, Condvar, Mutex},
    thread,
};

use symphonia::core::io::MediaSource;
use tracing::{debug, trace, warn};

use crate::common::types::AnyResult;

/// Size of each independently-fetched chunk. Larger chunks improve throughput
/// and reduce request overhead, especially for high-bitrate audio.
const CHUNK_SIZE: usize = 1024 * 1024; // 1 MB (was 128 KB)

/// How many chunks ahead of the read cursor to keep pre-fetched.
const PREFETCH_CHUNKS: usize = 16; // 16 MB window (was 32 * 128KB = 4MB)

/// Number of parallel download workers.
const MAX_CONCURRENT_FETCHES: usize = 3; // Reduced slightly to avoid YouTube rate limits

/// Timeout for each chunk fetch to prevent worker hangs.
const FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

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

pub struct SegmentedRemoteReader {
    pos: u64,
    len: u64,
    content_type: Option<String>,
    shared: Arc<(Mutex<ReaderState>, Condvar)>,
}

impl SegmentedRemoteReader {
    pub fn new(client: reqwest::Client, url: &str) -> AnyResult<Self> {
        let handle = tokio::runtime::Handle::current();

        let probe_req = client
            .get(url)
            .header("Range", "bytes=0-0")
            .header("Connection", "close")
            .timeout(std::time::Duration::from_secs(10))
            .send();

        let probe = handle.block_on(probe_req)?;

        let len = probe
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split('/').last())
            .and_then(|v| v.parse::<u64>().ok())
            .or_else(|| probe.content_length())
            .ok_or("SegmentedRemoteReader: could not determine content length")?;

        let content_type = probe
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);

        debug!(
            "Opened SegmentedRemoteReader: (len={}, type={:?})",
            len, content_type
        );

        // Mark chunk 0 as Empty so the first worker picks it up with high priority.
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

        // Spawn concurrent background workers — they race to fill chunk 0 first,
        // then fan out across the prefetch window.
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
            .timeout(FETCH_TIMEOUT)
            .send()
            .await?;

        if !res.status().is_success() && res.status() != reqwest::StatusCode::PARTIAL_CONTENT {
            return Err(format!("Fetch failed: status={}", res.status()).into());
        }
        Ok(res)
    }

    pub fn content_type(&self) -> Option<String> {
        self.content_type.clone()
    }
}

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
            let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
            if state.is_terminated {
                break;
            }

            let current_chunk_idx = (state.current_pos / CHUNK_SIZE as u64) as usize;

            // High priority: current read position chunk.
            let entry = state
                .chunks
                .entry(current_chunk_idx)
                .or_insert(ChunkState::Empty(0));
            if let ChunkState::Empty(_) = entry {
                *entry = ChunkState::Downloading;
                target_chunk_idx = Some(current_chunk_idx);
            } else {
                let cursor_ready = matches!(
                    state.chunks.get(&current_chunk_idx),
                    Some(ChunkState::Ready(_))
                );
                let window_limit = if cursor_ready { PREFETCH_CHUNKS } else { 2 };

                // Fill the prefetch window ahead of current position.
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
                // Nothing to fetch — wait for the read cursor to advance.
                let _ = cvar
                    .wait_timeout(state, std::time::Duration::from_millis(50))
                    .unwrap();
                continue;
            }
        }

        if let Some(idx) = target_chunk_idx {
            let offset = (idx * CHUNK_SIZE) as u64;
            let stream_len = lock.lock().unwrap_or_else(|e| e.into_inner()).total_len;
            let size = CHUNK_SIZE.min((stream_len - offset) as usize);

            trace!("Worker {}: Requesting chunk {} (offset={})", i, idx, offset);

            let fetch_fut = SegmentedRemoteReader::fetch_range(&client, &url, offset, size as u64);
            match handle.block_on(fetch_fut) {
                Ok(res) => {
                    let bytes_fut = res.bytes();
                    if let Ok(data) = handle.block_on(bytes_fut) {
                        let data = data.to_vec();
                        let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
                        let actual_len = data.len();
                        state.chunks.insert(idx, ChunkState::Ready(Arc::new(data)));
                        trace!("Worker {}: Filled chunk {} ({} bytes)", i, idx, actual_len);
                        cvar.notify_all();
                    } else {
                        warn!("Worker {}: Failed to read body for chunk {}", i, idx);
                        let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
                        state.chunks.insert(idx, ChunkState::Empty(0));
                        cvar.notify_all();
                    }
                }
                Err(e) => {
                    warn!("Worker {}: Fetch failed for chunk {}: {}", i, idx, e);
                    let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());

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

impl Read for SegmentedRemoteReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.pos >= self.len {
            return Ok(0);
        }

        let (lock, cvar) = &*self.shared;
        let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());

        // Keep workers informed of our current read position.
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
                        // Advance to next chunk boundary.
                        self.pos = ((chunk_idx + 1) * CHUNK_SIZE) as u64;
                        state.current_pos = self.pos;
                        continue;
                    }

                    let n = buf.len().min(available);
                    buf[..n].copy_from_slice(&data[offset_in_chunk..offset_in_chunk + n]);
                    self.pos += n as u64;
                    state.current_pos = self.pos;

                    // Evict old chunks to bound memory usage (keep 4 chunks behind cursor).
                    if chunk_idx > 8 {
                        state.chunks.retain(|&idx, _| idx >= chunk_idx - 4);
                    }
                    return Ok(n);
                }
                Some(ChunkState::Downloading) | Some(ChunkState::Empty(_)) => {
                    cvar.notify_all(); // Wake workers if they're sleeping

                    trace!("Waiting for chunk {}", chunk_idx);
                    let (new_state, _timeout) = cvar
                        .wait_timeout(state, std::time::Duration::from_millis(500))
                        .unwrap();
                    state = new_state;
                }
                None => {
                    // Chunk not yet queued — insert and wake workers.
                    state.chunks.insert(chunk_idx, ChunkState::Empty(0));
                    cvar.notify_all();
                }
            }
        }
    }
}

impl Seek for SegmentedRemoteReader {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        let new_pos = match pos {
            SeekFrom::Start(p) => p,
            SeekFrom::Current(delta) => self.pos.saturating_add_signed(delta),
            SeekFrom::End(delta) => self.len.saturating_add_signed(delta),
        };

        self.pos = new_pos.min(self.len);

        let (lock, cvar) = &*self.shared;
        let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
        state.current_pos = self.pos;
        cvar.notify_all();

        Ok(self.pos)
    }
}

impl MediaSource for SegmentedRemoteReader {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        Some(self.len)
    }
}

impl Drop for SegmentedRemoteReader {
    fn drop(&mut self) {
        let (lock, cvar) = &*self.shared;
        let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
        state.is_terminated = true;
        cvar.notify_all();
    }
}
