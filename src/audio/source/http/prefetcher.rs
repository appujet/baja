use std::{sync::Arc, time::Duration};

use bytes::Bytes;
use parking_lot::{Condvar, Mutex};
use tracing::{debug, warn};

use super::HttpSource;
use crate::audio::constants::{MAX_FETCH_RETRIES, MAX_HTTP_BUF_BYTES};

#[derive(Debug)]
pub enum PrefetchCommand {
    Continue,
    Seek(u64),
    Stop,
}

pub struct SharedState {
    pub chunks: std::collections::VecDeque<Bytes>,
    pub buffered: usize,
    pub done: bool,
    pub error: Option<String>,
    pub command: PrefetchCommand,
}

impl Default for SharedState {
    fn default() -> Self {
        Self::new()
    }
}

impl SharedState {
    pub fn new() -> Self {
        Self {
            chunks: std::collections::VecDeque::with_capacity(64),
            buffered: 0,
            done: false,
            error: None,
            command: PrefetchCommand::Continue,
        }
    }

    pub fn drain_into(&mut self, dst: &mut [u8]) -> usize {
        let mut written = 0;
        while written < dst.len() {
            let Some(front) = self.chunks.front_mut() else {
                break;
            };
            let want = dst.len() - written;
            let have = front.len();
            if have <= want {
                dst[written..written + have].copy_from_slice(front);
                written += have;
                self.buffered -= have;
                self.chunks.pop_front();
            } else {
                dst[written..].copy_from_slice(&front[..want]);
                *front = front.slice(want..);
                self.buffered -= want;
                written += want;
                break;
            }
        }
        written
    }

    pub fn skip(&mut self, mut n: usize) -> usize {
        let total = n;
        while n > 0 {
            let Some(front) = self.chunks.front_mut() else {
                break;
            };
            let have = front.len();
            if have <= n {
                n -= have;
                self.buffered -= have;
                self.chunks.pop_front();
            } else {
                *front = front.slice(n..);
                self.buffered -= n;
                n = 0;
            }
        }
        total - n
    }
}

const SLEEP_SLICE_MS: u64 = 50;

fn interruptible_sleep(shared: &Arc<(Mutex<SharedState>, Condvar)>, total_ms: u64) -> bool {
    let slices = (total_ms / SLEEP_SLICE_MS).max(1);
    for _ in 0..slices {
        std::thread::sleep(Duration::from_millis(SLEEP_SLICE_MS));
        if matches!(shared.0.lock().command, PrefetchCommand::Stop) {
            return true;
        }
    }
    false
}

pub fn prefetch_loop(
    shared: Arc<(Mutex<SharedState>, Condvar)>,
    client: reqwest::Client,
    url: String,
    mut current_pos: u64,
    mut response: Option<reqwest::Response>,
    total_len: Option<u64>,
    handle: tokio::runtime::Handle,
) {
    let mut retry_count: u32 = 0;

    'outer: loop {
        let seek_target: Option<u64> = {
            let (lock, cvar) = &*shared;
            let mut state = lock.lock();

            loop {
                match std::mem::replace(&mut state.command, PrefetchCommand::Continue) {
                    PrefetchCommand::Stop => break 'outer,
                    PrefetchCommand::Seek(pos) => {
                        state.done = false;
                        state.chunks.clear();
                        state.buffered = 0;
                        cvar.notify_all();
                        break Some(pos);
                    }
                    PrefetchCommand::Continue => {
                        if state.buffered >= MAX_HTTP_BUF_BYTES || state.done {
                            cvar.wait_for(&mut state, Duration::from_millis(200));
                            continue;
                        }
                        break None;
                    }
                }
            }
        };

        if let Some(target) = seek_target {
            let forward = target.saturating_sub(current_pos);
            if forward > 0 && forward <= 256 * 1024 && response.is_some() {
                debug!("prefetch: socket-skip {} bytes", forward);
                let mut leftover: Option<Bytes> = None;
                let res = response.take().unwrap();

                let skip_result = handle.block_on(async {
                    let mut res = res;
                    let mut remaining = forward;
                    while remaining > 0 {
                        match res.chunk().await {
                            Ok(Some(chunk)) => {
                                if chunk.len() as u64 <= remaining {
                                    remaining -= chunk.len() as u64;
                                } else {
                                    leftover = Some(chunk.slice(remaining as usize..));
                                    remaining = 0;
                                }
                            }
                            _ => return Err(()),
                        }
                    }
                    Ok(res)
                });

                match skip_result {
                    Ok(r) => {
                        current_pos = target;
                        response = Some(r);
                        if let Some(lo) = leftover {
                            let (lock, cvar) = &*shared;
                            let mut state = lock.lock();
                            state.buffered += lo.len();
                            state.chunks.push_front(lo);
                            cvar.notify_all();
                        }
                    }
                    Err(_) => {
                        current_pos = target;
                        response = None;
                    }
                }
            } else {
                current_pos = target;
                response = None;
            }
            retry_count = 0;
        }

        if response.is_none() {
            match handle.block_on(HttpSource::fetch_stream(&client, &url, current_pos, None)) {
                Ok(r) => {
                    response = Some(r);
                    retry_count = 0;
                }
                Err(e) => {
                    let msg = e.to_string();
                    if msg.contains("416") {
                        debug!("prefetch: 416 – reached end of stream");
                        let (lock, cvar) = &*shared;
                        let mut state = lock.lock();
                        state.done = true;
                        cvar.notify_all();
                        while state.done && matches!(state.command, PrefetchCommand::Continue) {
                            cvar.wait_for(&mut state, Duration::from_millis(200));
                        }
                        continue;
                    }

                    retry_count += 1;
                    if retry_count > MAX_FETCH_RETRIES {
                        warn!(
                            "prefetch: fetch failed fatally after {} retries: {}",
                            MAX_FETCH_RETRIES, e
                        );
                        let (lock, cvar) = &*shared;
                        let mut state = lock.lock();
                        state.error = Some(msg);
                        cvar.notify_all();
                        break 'outer;
                    }

                    let backoff_ms = 100u64 << (retry_count - 1).min(5);
                    warn!(
                        "prefetch: fetch failed (retry {}/{}): {} — backing off {}ms",
                        retry_count, MAX_FETCH_RETRIES, e, backoff_ms
                    );

                    if interruptible_sleep(&shared, backoff_ms) {
                        break 'outer;
                    }
                    continue;
                }
            }
        }

        {
            let (lock, cvar) = &*shared;
            let mut state = lock.lock();
            while state.buffered >= MAX_HTTP_BUF_BYTES
                && matches!(state.command, PrefetchCommand::Continue)
                && !state.done
            {
                cvar.wait_for(&mut state, Duration::from_millis(100));
            }
            if !matches!(state.command, PrefetchCommand::Continue) {
                continue;
            }
        }

        let res = response.as_mut().unwrap();
        match handle.block_on(res.chunk()) {
            Ok(Some(chunk)) => {
                let n = chunk.len();
                let (lock, cvar) = &*shared;
                let mut state = lock.lock();

                if !matches!(state.command, PrefetchCommand::Continue) {
                    continue;
                }

                current_pos += n as u64;
                state.buffered += n;
                state.chunks.push_back(chunk);
                cvar.notify_all();
            }
            Ok(None) => {
                response = None;
                retry_count = 0;

                let is_eof = total_len.is_none_or(|l| current_pos >= l);
                if is_eof {
                    let (lock, cvar) = &*shared;
                    let mut state = lock.lock();
                    state.done = true;
                    cvar.notify_all();
                    while state.done && matches!(state.command, PrefetchCommand::Continue) {
                        cvar.wait_for(&mut state, Duration::from_millis(200));
                    }
                }
            }
            Err(e) => {
                response = None;
                retry_count += 1;

                if retry_count > MAX_FETCH_RETRIES {
                    warn!(
                        "prefetch: read failed fatally after {} retries: {}",
                        MAX_FETCH_RETRIES, e
                    );
                    let (lock, cvar) = &*shared;
                    let mut state = lock.lock();
                    state.error = Some(e.to_string());
                    cvar.notify_all();
                    break 'outer;
                }

                let backoff_ms = 50u64 << (retry_count - 1).min(5);
                warn!(
                    "prefetch: read error (retry {}/{}): {} — backing off {}ms",
                    retry_count, MAX_FETCH_RETRIES, e, backoff_ms
                );

                if interruptible_sleep(&shared, backoff_ms) {
                    break 'outer;
                }
            }
        }
    }
}
