use std::{sync::Arc, thread, time::Duration};

use parking_lot::{Condvar, Mutex};
use tracing::{debug, warn};

use super::HttpSource;
use crate::audio::constants::{
    HTTP_FETCH_CHUNK_LIMIT, HTTP_PREFETCH_BUFFER_SIZE, HTTP_SOCKET_SKIP_LIMIT,
};

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
                Some(HTTP_FETCH_CHUNK_LIMIT),
            )) {
                Ok(res) => current_response = Some(res),
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

                    warn!("HttpSource prefetch fetch failed: {}", e);
                    thread::sleep(Duration::from_millis(500));
                    continue;
                }
            }
        }

        if let Some(res) = &mut current_response {
            match handle.block_on(res.chunk()) {
                Ok(Some(bytes)) => {
                    let n = bytes.len();
                    let (lock, cvar) = &*shared;
                    let mut state = lock.lock();

                    if matches!(state.command, PrefetchCommand::Continue) {
                        state.next_buf.extend_from_slice(&bytes);
                        current_pos += n as u64;
                        if state.next_buf.len() >= HTTP_PREFETCH_BUFFER_SIZE {
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
                    thread::sleep(Duration::from_millis(100));
                    continue;
                }
            }
        }
    }
}
