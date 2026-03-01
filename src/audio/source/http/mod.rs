use std::{
    io::{Read, Seek, SeekFrom},
    sync::Arc,
    thread,
};

use parking_lot::{Condvar, Mutex};
use symphonia::core::io::MediaSource;
use tracing::{debug, info};

use super::AudioSource;
use crate::{audio::constants::HTTP_INITIAL_BUF_CAPACITY, common::types::AnyResult};

pub mod prefetcher;
use prefetcher::{PrefetchCommand, SharedState, prefetch_loop};

/// Streaming HTTP source with a dedicated prefetch thread.
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
                next_buf: Vec::with_capacity(HTTP_INITIAL_BUF_CAPACITY),
                done: false,
                need_data: true,
                command: PrefetchCommand::Continue,
            }),
            Condvar::new(),
        ));

        let shared_clone = Arc::clone(&shared);
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
            buf: Vec::with_capacity(HTTP_INITIAL_BUF_CAPACITY),
            buf_pos: 0,
            shared,
        })
    }

    pub(crate) async fn fetch_stream(
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

impl AudioSource for HttpSource {
    fn content_type(&self) -> Option<String> {
        self.content_type.clone()
    }
}

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
