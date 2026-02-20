use crate::common::types::AnyResult;
pub mod ua;

use std::{
    io::{Read, Seek, SeekFrom},
    sync::{Arc, Condvar, Mutex},
    thread,
};

use symphonia::core::io::MediaSource;
use tracing::{debug, info, warn};

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

pub struct RemoteReader {
    pos: u64,
    len: Option<u64>,
    content_type: Option<String>,

    buf: Vec<u8>,
    buf_pos: usize,
    shared: Arc<(Mutex<SharedState>, Condvar)>,
}

impl RemoteReader {
    pub fn new(
        url: &str,
        local_addr: Option<std::net::IpAddr>,
        proxy: Option<crate::configs::HttpProxyConfig>,
    ) -> AnyResult<Self> {
        let user_agent = ua::get_youtube_ua(url)
            .map(str::to_string)
            .unwrap_or_else(crate::common::http::HttpClient::random_user_agent);

        let mut builder = reqwest::blocking::Client::builder()
            .user_agent(user_agent)
            .timeout(std::time::Duration::from_secs(15));

        if let Some(ip) = local_addr {
            builder = builder.local_address(ip);
        }

        if let Some(proxy_config) = proxy {
            if let Some(p_url) = &proxy_config.url {
                if let Ok(mut proxy_obj) = reqwest::Proxy::all(p_url) {
                    if let (Some(u), Some(p)) = (proxy_config.username, proxy_config.password) {
                        proxy_obj = proxy_obj.basic_auth(&u, &p);
                    }
                    builder = builder.proxy(proxy_obj);
                    debug!("Configured proxy for RemoteReader: {}", p_url);
                }
            }
        }

        let client = builder.build()?;

        // Blocking initial fetch to extract Content-Length early.
        let response = Self::fetch_stream(&client, url, 0)?;
        let len = response.content_length();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);

        info!("Opened RemoteReader: {} (len={:?})", url, len);

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

        thread::Builder::new()
            .name("remote-prefetch".to_string())
            .spawn(move || {
                prefetch_loop(shared_clone, client, url_clone, 0, Some(response));
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

    /// Internal helper to perform a Range request.
    fn fetch_stream(
        client: &reqwest::blocking::Client,
        url: &str,
        offset: u64,
    ) -> AnyResult<reqwest::blocking::Response> {
        let mut req = client
            .get(url)
            .header("Accept", "*/*")
            .header("Accept-Encoding", "identity");

        if offset > 0 {
            req = req.header("Range", format!("bytes={}-", offset));
        }

        let res = req.send()?;
        if !res.status().is_success() {
            return Err(format!("Stream fetch failed ({}): {}", res.status(), url).into());
        }
        Ok(res)
    }

    pub fn content_type(&self) -> Option<String> {
        self.content_type.clone()
    }
}

fn prefetch_loop(
    shared: Arc<(Mutex<SharedState>, Condvar)>,
    client: reqwest::blocking::Client,
    url: String,
    mut current_pos: u64,
    mut current_response: Option<reqwest::blocking::Response>,
) {
    let mut chunk = vec![0u8; 128 * 1024];

    loop {
        let mut target_seek = None;

        // 1. Check commands
        {
            let (lock, cvar) = &*shared;
            let mut state = lock.lock().unwrap();

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
                    {
                        state = cvar.wait(state).unwrap();
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

        // Apply Seek
        if let Some(pos) = target_seek {
            let forward_jump = if pos > current_pos {
                pos - current_pos
            } else {
                0
            };

            // Optimization: if small forward jump, skip the socket to avoid TCP teardown (~300ms latency)
            if forward_jump > 0 && forward_jump <= 1_000_000 && current_response.is_some() {
                debug!(
                    "RemoteReader prefetch thread socket-skipping {} bytes",
                    forward_jump
                );
                let mut discard = std::io::sink();
                let mut res = current_response.take().unwrap();

                if let Ok(copied) = std::io::copy(&mut (&mut res).take(forward_jump), &mut discard)
                {
                    if copied == forward_jump {
                        current_pos = pos;
                        current_response = Some(res); // Keep it alive
                    } else {
                        current_response = None; // Failed to skip, force reconnect
                    }
                } else {
                    current_response = None;
                }
            } else {
                current_pos = pos;
                current_response = None; // Force reconnect
            }
        }

        // 2. Ensure connection
        if current_response.is_none() {
            match RemoteReader::fetch_stream(&client, &url, current_pos) {
                Ok(res) => current_response = Some(res),
                Err(e) => {
                    warn!("RemoteReader prefetch fetch failed: {}", e);
                    thread::sleep(std::time::Duration::from_millis(500));
                    continue;
                }
            }
        }

        // 3. Read up to Chunk size
        let mut read_bytes = 0;
        if let Some(res) = &mut current_response {
            match res.read(&mut chunk) {
                Ok(0) => {
                    let (lock, cvar) = &*shared;
                    let mut state = lock.lock().unwrap();
                    state.done = true;
                    cvar.notify_all();

                    // Wait for new commands
                    while state.done && matches!(state.command, PrefetchCommand::Continue) {
                        state = cvar.wait(state).unwrap();
                    }
                    continue;
                }
                Ok(n) => {
                    read_bytes = n;
                    current_pos += n as u64;
                }
                Err(e) => {
                    warn!("RemoteReader prefetch read failed: {}", e);
                    current_response = None;
                    thread::sleep(std::time::Duration::from_millis(100));
                    continue;
                }
            }
        }

        // 4. Push to shared buffer
        if read_bytes > 0 {
            let (lock, cvar) = &*shared;
            let mut state = lock.lock().unwrap();

            // If interrupted by a seek, drop this batch
            if !matches!(state.command, PrefetchCommand::Continue) {
                continue;
            }

            state.next_buf.extend_from_slice(&chunk[..read_bytes]);

            // Pause fetching if we have 2MB buffered ahead
            if state.next_buf.len() >= 2 * 1024 * 1024 {
                state.need_data = false;
            }
            cvar.notify_all();
        }
    }
}

impl Read for RemoteReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        // Serve from active buffer
        if self.buf_pos < self.buf.len() {
            let n = std::cmp::min(buf.len(), self.buf.len() - self.buf_pos);
            buf[..n].copy_from_slice(&self.buf[self.buf_pos..self.buf_pos + n]);
            self.buf_pos += n;
            self.pos += n as u64;
            return Ok(n);
        }

        // Active buffer exhausted — pull from background thread
        let (lock, cvar) = &*self.shared;
        let mut state = lock.lock().unwrap();

        state.need_data = true;
        cvar.notify_one();

        while state.next_buf.is_empty() && !state.done {
            state = cvar.wait(state).unwrap();
        }

        if state.next_buf.is_empty() && state.done {
            return Ok(0); // EOF
        }

        // Instant swap
        self.buf.clear();
        self.buf_pos = 0;
        std::mem::swap(&mut self.buf, &mut state.next_buf);

        state.need_data = true;
        cvar.notify_one();
        drop(state);

        self.read(buf)
    }
}

impl Seek for RemoteReader {
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

        if new_pos != self.pos {
            let forward_jump = if new_pos > self.pos {
                new_pos - self.pos
            } else {
                0
            };

            let (lock, cvar) = &*self.shared;
            let mut state = lock.lock().unwrap();

            let buf_remaining = self.buf.len() as u64 - self.buf_pos as u64;
            let next_buf_remaining = state.next_buf.len() as u64;

            // Pure in-memory jump if we already downloaded the requested offset!
            if forward_jump > 0 && forward_jump <= (buf_remaining + next_buf_remaining) {
                debug!(
                    "RemoteReader purely in-memory seek forward by {} bytes",
                    forward_jump
                );
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

            // Outside memory: delegate to background thread
            debug!("RemoteReader hard seek: {} -> {}", self.pos, new_pos);
            self.buf.clear();
            self.buf_pos = 0;
            self.pos = new_pos;

            state.command = PrefetchCommand::Seek(new_pos);
            state.next_buf.clear();
            state.done = false;
            state.need_data = true;
            cvar.notify_all();
        }
        Ok(self.pos)
    }
}

impl MediaSource for RemoteReader {
    fn is_seekable(&self) -> bool {
        self.len.is_some()
    }

    fn byte_len(&self) -> Option<u64> {
        self.len
    }
}

impl Drop for RemoteReader {
    fn drop(&mut self) {
        let (lock, cvar) = &*self.shared;
        let mut state = lock.lock().unwrap();
        state.command = PrefetchCommand::Stop;
        state.need_data = true;
        cvar.notify_all();
    }
}
