use std::{
    io::{Read, Seek, SeekFrom},
    sync::Arc,
    thread,
};

use parking_lot::{Condvar, Mutex};
use symphonia::core::io::MediaSource;
use tracing::debug;

use super::AudioSource;
use crate::common::types::AnyResult;

pub mod prefetcher;
use prefetcher::{PrefetchCommand, SharedState, prefetch_loop};

pub struct HttpSource {
    pos: u64,
    len: Option<u64>,
    content_type: Option<String>,
    shared: Arc<(Mutex<SharedState>, Condvar)>,
}

impl HttpSource {
    pub fn new(client: reqwest::Client, url: &str) -> AnyResult<Self> {
        let handle = tokio::runtime::Handle::current();
        let response = handle.block_on(Self::fetch_stream(&client, url, 0, None))?;

        let len = response
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.split('/').next_back())
            .and_then(|s| s.parse::<u64>().ok())
            .or_else(|| response.content_length());

        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);

        debug!("HttpSource opened: {} (len={:?})", url, len);

        let shared = Arc::new((Mutex::new(SharedState::new()), Condvar::new()));
        let shared_clone = Arc::clone(&shared);
        let url_clone = url.to_string();
        let handle_clone = handle.clone();

        thread::Builder::new()
            .name("http-prefetch".into())
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
            .header("Range", &range)
            .send()
            .await?;

        if !res.status().is_success() {
            return Err(format!("HTTP {} for {}", res.status(), url).into());
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
        let (lock, cvar) = &*self.shared;
        let mut state = lock.lock();

        loop {
            if !state.chunks.is_empty() || state.done || state.error.is_some() {
                break;
            }
            cvar.wait(&mut state);
        }

        if let Some(err) = state.error.take() {
            return Err(std::io::Error::other(err));
        }

        let n = state.drain_into(buf);

        if state.buffered < crate::audio::constants::HTTP_PREFETCH_BUFFER_SIZE {
            cvar.notify_one();
        }

        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for HttpSource {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        let new_pos = match pos {
            SeekFrom::Start(p) => p,
            SeekFrom::Current(d) => self.pos.saturating_add_signed(d),
            SeekFrom::End(d) => {
                let l = self.len.ok_or_else(|| {
                    std::io::Error::new(std::io::ErrorKind::Unsupported, "stream length unknown")
                })?;
                l.saturating_add_signed(d)
            }
        };

        if new_pos == self.pos {
            return Ok(self.pos);
        }

        let (lock, cvar) = &*self.shared;
        let mut state = lock.lock();

        let forward = new_pos.saturating_sub(self.pos);
        if forward > 0 && forward <= state.buffered as u64 {
            debug!("HttpSource: in-memory seek +{} bytes", forward);
            state.skip(forward as usize);
            self.pos = new_pos;
            return Ok(self.pos);
        }

        debug!("HttpSource: hard seek {} → {}", self.pos, new_pos);
        state.chunks.clear();
        state.buffered = 0;
        state.done = false;
        state.error = None;
        state.command = PrefetchCommand::Seek(new_pos);
        cvar.notify_all();

        self.pos = new_pos;
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
        cvar.notify_all();
    }
}
