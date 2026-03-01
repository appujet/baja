use std::{sync::Arc, time::Duration};

use parking_lot::{Condvar, Mutex};
use tracing::{debug, warn};

use super::HttpSource;
use crate::audio::constants::{
    HTTP_PREFETCH_BUFFER_SIZE, HTTP_SOCKET_SKIP_LIMIT, MAX_FETCH_RETRIES, MAX_HTTP_BUF_BYTES,
};

const CHUNK_READ_TIMEOUT: Duration = Duration::from_secs(5);

const GOOD_CHUNKS_BEFORE_RESET: u32 = 8;

#[derive(Debug)]
pub enum PrefetchCommand {
    Continue,
    Seek(u64),
    Stop,
}

pub struct SharedState {
    pub next_buf: Vec<u8>,
    pub done: bool,
    pub need_data: bool,
    pub command: PrefetchCommand,
    pub error: Option<String>,
}

const SLEEP_SLICE_MS: u64 = 50;

fn interruptible_sleep(shared: &Arc<(Mutex<SharedState>, Condvar)>, total_ms: u64) -> bool /* returns true if Stop was received */
{
    let slices = (total_ms / SLEEP_SLICE_MS).max(1);
    for _ in 0..slices {
        std::thread::sleep(Duration::from_millis(SLEEP_SLICE_MS));
        let (lock, _) = &**shared;
        let state = lock.lock();
        if matches!(state.command, PrefetchCommand::Stop) {
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
    mut current_response: Option<reqwest::Response>,
    total_len: Option<u64>,
    handle: tokio::runtime::Handle,
) {
    let mut retry_count: u32 = 0;
    let mut good_chunks: u32 = 0;
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
                        && state.next_buf.len() >= HTTP_PREFETCH_BUFFER_SIZE
                    {
                        cvar.wait_for(&mut state, Duration::from_millis(500));
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

        if let Some(pos) = target_seek {
            let forward_jump = pos.saturating_sub(current_pos);

            if forward_jump > 0
                && forward_jump <= HTTP_SOCKET_SKIP_LIMIT
                && current_response.is_some()
            {
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

        if current_response.is_none() {
            match handle.block_on(HttpSource::fetch_stream(
                &client,
                &url,
                current_pos,
                None,
            )) {
                Ok(res) => {
                    current_response = Some(res);
                    good_chunks = 0;
                }
                Err(e) => {
                    let err_msg = e.to_string();
                    if err_msg.contains("416") {
                        debug!(
                            "HttpSource prefetch: hit end of stream (416 Range Not Satisfiable)"
                        );
                        let (lock, cvar) = &*shared;
                        let mut state = lock.lock();
                        state.done = true;
                        cvar.notify_all();
                        break;
                    }

                    retry_count += 1;
                    if retry_count <= MAX_FETCH_RETRIES {
                        let backoff_ms = 200u64 * (1u64 << (retry_count - 1).min(4));
                        debug!(
                            "HttpSource prefetch fetch failed (retry {}/{}), backoff {}ms: {}",
                            retry_count, MAX_FETCH_RETRIES, backoff_ms, e
                        );
                        if interruptible_sleep(&shared, backoff_ms) {
                            break;
                        }
                        continue;
                    }

                    warn!(
                        "HttpSource prefetch fetch failed fatally after {} retries: {}",
                        retry_count, e
                    );
                    let (lock, cvar) = &*shared;
                    let mut state = lock.lock();
                    state.error = Some(e.to_string());
                    cvar.notify_all();
                    break;
                }
            }
        }

        if let Some(res) = &mut current_response {
            let chunk_result = handle
                .block_on(async { tokio::time::timeout(CHUNK_READ_TIMEOUT, res.chunk()).await });

            match chunk_result {
                Err(_elapsed) => {
                    debug!(
                        "HttpSource prefetch: chunk read timed out after {}s, reconnecting",
                        CHUNK_READ_TIMEOUT.as_secs()
                    );
                    current_response = None;
                    retry_count += 1;
                    good_chunks = 0;
                    if retry_count > MAX_FETCH_RETRIES {
                        warn!(
                            "HttpSource prefetch timed out fatally after {} retries",
                            retry_count
                        );
                        let (lock, cvar) = &*shared;
                        let mut state = lock.lock();
                        state.error = Some("Stream timed out repeatedly".into());
                        cvar.notify_all();
                        break;
                    }
                    let backoff_ms = 100u64 * (1u64 << (retry_count - 1).min(4));
                    if interruptible_sleep(&shared, backoff_ms) {
                        break;
                    }
                }

                Ok(Ok(Some(bytes))) => {
                    let n = bytes.len();
                    let (lock, cvar) = &*shared;
                    let mut state = lock.lock();

                    if matches!(state.command, PrefetchCommand::Continue) {
                        if state.next_buf.len() < MAX_HTTP_BUF_BYTES {
                            state.next_buf.extend_from_slice(&bytes);
                            current_pos += n as u64;
                        }
                        if state.next_buf.len() >= HTTP_PREFETCH_BUFFER_SIZE {
                            state.need_data = false;
                        }
                        cvar.notify_all();
                    }

                    good_chunks += 1;
                    if good_chunks >= GOOD_CHUNKS_BEFORE_RESET {
                        retry_count = 0;
                        good_chunks = 0;
                    }
                }

                Ok(Ok(None)) => {
                    let (lock, cvar) = &*shared;
                    let mut state = lock.lock();

                    let is_eof = total_len.map_or(true, |l| current_pos >= l);
                    if is_eof {
                        state.done = true;
                        cvar.notify_all();
                        while state.done && matches!(state.command, PrefetchCommand::Continue) {
                            cvar.wait_for(&mut state, Duration::from_millis(500));
                        }
                    } else {
                        current_response = None;
                        good_chunks = 0;
                    }
                    continue;
                }

                Ok(Err(e)) => {
                    current_response = None;
                    good_chunks = 0;
                    retry_count += 1;
                    if retry_count <= MAX_FETCH_RETRIES {
                        let backoff_ms = 100u64 * (1u64 << (retry_count - 1).min(4));
                        debug!(
                            "HttpSource prefetch read failed (retry {}/{}), backoff {}ms: {}",
                            retry_count, MAX_FETCH_RETRIES, backoff_ms, e
                        );
                        if interruptible_sleep(&shared, backoff_ms) {
                            break;
                        }
                        continue;
                    }

                    warn!(
                        "HttpSource prefetch read failed fatally after {} retries: {}",
                        retry_count, e
                    );
                    let (lock, cvar) = &*shared;
                    let mut state = lock.lock();
                    state.error = Some(e.to_string());
                    cvar.notify_all();
                    break;
                }
            }
        }
    }
}
