//! `HttpSource` — prefetch-thread HTTP audio source.
//!
//! Replaces `audio::remote_reader::BaseRemoteReader` with a cleaner name
//! and implements the [`AudioSource`](super::AudioSource) trait.
//!
//! # How it works
//!
//! ```text
//!  ┌─────────────────────┐    Range requests
//!  │  prefetch thread    │ ◄──────────────────── server
//!  │  (HTTP streaming)   │
//!  └────────┬────────────┘
//!           │ shared ring-buffer (up to 8 MB ahead)
//!  ┌────────▼────────────┐
//!  │   HttpSource        │ ◄── Read / Seek (called by Symphonia)
//!  └─────────────────────┘
//! ```
//!
//! Seek strategy:
//! * **In-memory forward seek** — served instantly from buffered data.
//! * **Small forward gap** (≤ 1 MB) — socket-skip on the live connection.
//! * **Hard seek** — new Range request from the target offset.

use std::{
    io::{Read, Seek, SeekFrom},
    sync::Arc,
    thread,
};

use parking_lot::{Condvar, Mutex};
use symphonia::core::io::MediaSource;
use tracing::{debug, info, warn};

use crate::common::types::AnyResult;

use super::AudioSource;

// ─── Internal shared state ────────────────────────────────────────────────────

#[derive(Debug)]
enum PrefetchCommand {
    Continue,
    Seek(u64),
    Stop,
}

struct SharedState {
    next_buf: Vec<u8>,
    done: bool,
    need_data: bool,
    command: PrefetchCommand,
}

// ─── HttpSource ───────────────────────────────────────────────────────────────

/// Streaming HTTP source with a dedicated prefetch thread.
///
/// Renamed from `BaseRemoteReader` — same logic, cleaner API.
pub struct HttpSource {
    pos: u64,
    len: Option<u64>,
    content_type: Option<String>,
    buf: Vec<u8>,
    buf_pos: usize,
    shared: Arc<(Mutex<SharedState>, Condvar)>,
}

impl HttpSource {
    /// Open a URL and start the prefetch thread immediately.
    pub fn new(client: reqwest::Client, url: &str) -> AnyResult<Self> {
        let handle = tokio::runtime::Handle::current();
        let response = handle.block_on(Self::fetch_stream(&client, url, 0, None))?;

        let content_range_len = response
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split('/').last())
            .and_then(|v| v.parse::<u64>().ok());

        let len = content_range_len.or_else(|| response.content_length());

        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);

        info!("Opened HttpSource: {} (len={:?})", url, len);

        let shared = Arc::new((
            Mutex::new(SharedState {
                next_buf: Vec::with_capacity(256 * 1024),
                done: false,
                need_data: true,
                command: PrefetchCommand::Continue,
            }),
            Condvar::new(),
        ));

        let shared_clone = shared.clone();
        let url_clone = url.to_string();
        let handle_clone = handle.clone();

        thread::Builder::new()
            .name("http-source-prefetch".to_string())
            .spawn(move || {
                prefetch_loop(
                    shared_clone,
                    client,
                    url_clone,
                    0,
                    Some(response),
                    len,
                    handle_clone,
                );
            })?;

        Ok(Self {
            pos: 0,
            len,
            content_type,
            buf: Vec::with_capacity(256 * 1024),
            buf_pos: 0,
            shared,
        })
    }

    async fn fetch_stream(
        client: &reqwest::Client,
        url: &str,
        offset: u64,
        limit: Option<u64>,
    ) -> AnyResult<reqwest::Response> {
        let range = match limit {
            Some(l) => format!("bytes={}-{}", offset, offset + l - 1),
            None => format!("bytes={}-", offset),
        };

        let res = client
            .get(url)
            .header("Accept", "*/*")
            .header("Accept-Encoding", "identity")
            .header("Connection", "keep-alive")
            .header("Range", range)
            .send()
            .await?;

        if !res.status().is_success() {
            return Err(format!("Stream fetch failed ({}): {}", res.status(), url).into());
        }
        Ok(res)
    }
}

// ─── AudioSource ─────────────────────────────────────────────────────────────

impl AudioSource for HttpSource {
    fn content_type(&self) -> Option<String> {
        self.content_type.clone()
    }
}

// ─── Read / Seek / MediaSource ────────────────────────────────────────────────

impl Read for HttpSource {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.buf_pos < self.buf.len() {
            let n = std::cmp::min(buf.len(), self.buf.len() - self.buf_pos);
            buf[..n].copy_from_slice(&self.buf[self.buf_pos..self.buf_pos + n]);
            self.buf_pos += n;
            self.pos += n as u64;
            return Ok(n);
        }

        let (lock, cvar) = &*self.shared;
        let mut state = lock.lock();
        state.need_data = true;
        cvar.notify_one();

        while state.next_buf.is_empty() && !state.done {
            cvar.wait(&mut state);
        }

        if state.next_buf.is_empty() && state.done {
            return Ok(0);
        }

        self.buf.clear();
        self.buf_pos = 0;
        std::mem::swap(&mut self.buf, &mut state.next_buf);
        state.need_data = true;
        cvar.notify_one();
        drop(state);

        self.read(buf)
    }
}

impl Seek for HttpSource {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        let new_pos = match pos {
            SeekFrom::Start(p) => p,
            SeekFrom::Current(delta) => self.pos.saturating_add_signed(delta),
            SeekFrom::End(delta) => {
                let len = self.len.ok_or_else(|| {
                    std::io::Error::new(std::io::ErrorKind::Unsupported, "Stream length unknown")
                })?;
                len.saturating_add_signed(delta)
            }
        };

        if new_pos == self.pos {
            return Ok(self.pos);
        }

        let forward_jump = new_pos.saturating_sub(self.pos);

        let (lock, cvar) = &*self.shared;
        let mut state = lock.lock();

        let buf_remaining = self.buf.len() as u64 - self.buf_pos as u64;
        let next_buf_remaining = state.next_buf.len() as u64;

        // In-memory forward seek.
        if forward_jump > 0 && forward_jump <= (buf_remaining + next_buf_remaining) {
            debug!("HttpSource: in-memory seek +{} bytes", forward_jump);
            if forward_jump <= buf_remaining {
                self.buf_pos += forward_jump as usize;
            } else {
                let next_jump = forward_jump - buf_remaining;
                self.buf.clear();
                self.buf_pos = 0;
                std::mem::swap(&mut self.buf, &mut state.next_buf);
                self.buf_pos = next_jump as usize;
                state.need_data = true;
                cvar.notify_one();
            }
            self.pos = new_pos;
            return Ok(self.pos);
        }

        // Hard seek — delegate to prefetch thread.
        debug!("HttpSource: hard seek {} → {}", self.pos, new_pos);
        self.buf.clear();
        self.buf_pos = 0;
        self.pos = new_pos;
        state.command = PrefetchCommand::Seek(new_pos);
        state.next_buf.clear();
        state.done = false;
        state.need_data = true;
        cvar.notify_all();

        Ok(self.pos)
    }
}

impl MediaSource for HttpSource {
    fn is_seekable(&self) -> bool {
        self.len.is_some()
    }

    fn byte_len(&self) -> Option<u64> {
        self.len
    }
}

impl Drop for HttpSource {
    fn drop(&mut self) {
        let (lock, cvar) = &*self.shared;
        let mut state = lock.lock();
        state.command = PrefetchCommand::Stop;
        state.need_data = true;
        cvar.notify_all();
    }
}

// ─── Prefetch loop ────────────────────────────────────────────────────────────

fn prefetch_loop(
    shared: Arc<(Mutex<SharedState>, Condvar)>,
    client: reqwest::Client,
    url: String,
    mut current_pos: u64,
    mut current_response: Option<reqwest::Response>,
    total_len: Option<u64>,
    handle: tokio::runtime::Handle,
) {
    loop {
        let mut target_seek = None;

        {
            let (lock, cvar) = &*shared;
            let mut state = lock.lock();

            match std::mem::replace(&mut state.command, PrefetchCommand::Continue) {
                PrefetchCommand::Seek(pos) => {
                    target_seek = Some(pos);
                    state.done = false;
                    state.next_buf.clear();
                    state.need_data = true;
                }
                PrefetchCommand::Stop => break,
                PrefetchCommand::Continue => {
                    while !state.need_data
                        && !state.done
                        && matches!(state.command, PrefetchCommand::Continue)
                        && state.next_buf.len() >= 8 * 1024 * 1024
                    {
                        cvar.wait(&mut state);
                    }
                    if matches!(state.command, PrefetchCommand::Stop) {
                        break;
                    }
                    if let PrefetchCommand::Seek(pos) =
                        std::mem::replace(&mut state.command, PrefetchCommand::Continue)
                    {
                        target_seek = Some(pos);
                        state.done = false;
                        state.next_buf.clear();
                        state.need_data = true;
                    }
                }
            }
        }

        // Apply seek.
        if let Some(pos) = target_seek {
            let forward_jump = pos.saturating_sub(current_pos);

            // Socket-skip for small forward jumps (avoids TCP teardown ~300 ms).
            if forward_jump > 0 && forward_jump <= 1_000_000 && current_response.is_some() {
                debug!("HttpSource prefetch: socket-skip {} bytes", forward_jump);
                let mut res = current_response.take().unwrap();
                let mut leftovers = Vec::new();

                let res_result = handle.block_on(async {
                    let mut skipped = 0u64;
                    while skipped < forward_jump {
                        match res.chunk().await {
                            Ok(Some(c)) => {
                                let take = (forward_jump - skipped).min(c.len() as u64);
                                skipped += take;
                                if take < c.len() as u64 {
                                    leftovers.extend_from_slice(&c[take as usize..]);
                                }
                            }
                            _ => return Err(()),
                        }
                    }
                    Ok(res)
                });

                if let Ok(fixed) = res_result {
                    current_pos = pos;
                    current_response = Some(fixed);
                    if !leftovers.is_empty() {
                        let (lock, cvar) = &*shared;
                        let mut state = lock.lock();
                        state.next_buf.extend_from_slice(&leftovers);
                        cvar.notify_all();
                    }
                } else {
                    current_response = None;
                }
            } else {
                current_pos = pos;
                current_response = None;
            }
        }

        // Ensure connection (request in 5 MB chunks to avoid throttling).
        if current_response.is_none() {
            let chunk_limit = 5 * 1024 * 1024;
            match handle.block_on(HttpSource::fetch_stream(
                &client,
                &url,
                current_pos,
                Some(chunk_limit),
            )) {
                Ok(res) => current_response = Some(res),
                Err(e) => {
                    warn!("HttpSource prefetch fetch failed: {}", e);
                    thread::sleep(std::time::Duration::from_millis(500));
                    continue;
                }
            }
        }

        // Read chunk.
        if let Some(res) = &mut current_response {
            match handle.block_on(res.chunk()) {
                Ok(Some(bytes)) => {
                    let n = bytes.len();
                    let (lock, cvar) = &*shared;
                    let mut state = lock.lock();

                    if matches!(state.command, PrefetchCommand::Continue) {
                        state.next_buf.extend_from_slice(&bytes);
                        current_pos += n as u64;
                        if state.next_buf.len() >= 8 * 1024 * 1024 {
                            state.need_data = false;
                        }
                        cvar.notify_all();
                    }
                }
                Ok(None) => {
                    let (lock, cvar) = &*shared;
                    let mut state = lock.lock();

                    let is_eof = total_len.map_or(true, |l| current_pos >= l);
                    if is_eof {
                        state.done = true;
                        cvar.notify_all();
                        while state.done && matches!(state.command, PrefetchCommand::Continue) {
                            cvar.wait(&mut state);
                        }
                    } else {
                        current_response = None;
                    }
                    continue;
                }
                Err(e) => {
                    warn!("HttpSource prefetch read failed: {}", e);
                    current_response = None;
                    thread::sleep(std::time::Duration::from_millis(100));
                    continue;
                }
            }
        }
    }
}
