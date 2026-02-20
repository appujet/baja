pub mod fetcher;
pub mod parser;
pub mod resolver;
pub mod ts_demux;
pub mod types;
pub mod utils;

use crate::configs::HttpProxyConfig;
use self::fetcher::fetch_segment_into;
use self::resolver::{resolve_playlist, resolve_url_string};
use self::ts_demux::extract_adts_from_ts;
use self::types::Resource;
use crate::sources::youtube::cipher::YouTubeCipherManager;
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::{Arc, Condvar, Mutex};
use symphonia::core::io::MediaSource;

/// Number of segments fetched into the look-ahead buffer before the reader
/// swaps it into the active position.  Keep this small so swaps are fast but
/// large enough that network jitter is absorbed.
const PREFETCH_SEGMENTS: usize = 3;

/// Low-water mark: when the active buffer has fewer bytes remaining than this,
/// we wake the background thread early so it can start filling the next buffer.
const LOW_WATER_BYTES: usize = 64 * 1024; // 64 KiB

// ──────────────────── Shared state between reader & prefetcher ────────────────

/// Commands sent from the reader thread to the prefetch thread.
enum PrefetchCommand {
    /// Continue normal sequential fetching.
    Continue,
    /// Seek: discard current work and restart from this segment index.
    Seek(usize),
    /// Shut down the background thread.
    Stop,
}

struct SharedState {
    /// The "next" buffer – filled by the background thread.
    next_buf: Vec<u8>,
    /// True when the reader thread needs the background thread to fill data.
    need_data: bool,
    /// Pending segments the background thread should fetch from.
    pending: Vec<Resource>,
    /// Current segment index (tracks progress through all_segments).
    current_segment_index: usize,
    /// Command from reader → background thread.
    command: PrefetchCommand,
    /// True when background thread has finished processing a seek.
    seek_done: bool,
    /// True when there are no more segments to fetch (end of stream).
    eos: bool,
}

// ──────────────────── HlsReader ──────────────────────────────────────────────

pub struct HlsReader {
    /// Active buffer being consumed by `read()`.
    buf: Vec<u8>,
    /// Read cursor inside `buf`.
    pos: usize,

    /// Shared mutable state protected by a mutex + condvar.
    shared: Arc<(Mutex<SharedState>, Condvar)>,
    /// Handle to the background prefetch thread (joined on drop).
    bg_thread: Option<std::thread::JoinHandle<()>>,

    /// All segments (kept for seeking).
    all_segments: Vec<Resource>,
    /// Segment durations in seconds (parallel to all_segments).
    segment_durations: Vec<f64>,
    /// Whether segment durations are available (enables seeking).
    has_durations: bool,
}

impl Drop for HlsReader {
    fn drop(&mut self) {
        // Signal the background thread to stop.
        {
            let (lock, cvar) = &*self.shared;
            let mut state = lock.lock().unwrap();
            state.command = PrefetchCommand::Stop;
            state.need_data = true;
            cvar.notify_one();
        }
        if let Some(handle) = self.bg_thread.take() {
            let _ = handle.join();
        }
    }
}

impl HlsReader {
    pub fn new(
        manifest_url: &str,
        local_addr: Option<std::net::IpAddr>,
        cipher_manager: Option<Arc<YouTubeCipherManager>>,
        player_url: Option<String>,
        proxy: Option<HttpProxyConfig>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let mut builder = reqwest::blocking::Client::builder()
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.0.0 Safari/537.36")
            .timeout(std::time::Duration::from_secs(15));

        if let Some(ip) = local_addr {
            builder = builder.local_address(ip);
        }

        if let Some(ref proxy_cfg) = proxy {
            if let Some(ref proxy_url) = proxy_cfg.url {
                tracing::debug!("HLS: configuring proxy: {}", proxy_url);
                let mut proxy_obj = reqwest::Proxy::all(proxy_url)?;
                if let (Some(user), Some(pass)) = (&proxy_cfg.username, &proxy_cfg.password) {
                    proxy_obj = proxy_obj.basic_auth(user, pass);
                }
                builder = builder.proxy(proxy_obj);
            }
        }

        let client = builder.build()?;
        let (segment_urls, map_url) = resolve_playlist(&client, manifest_url)?;

        if segment_urls.is_empty() {
            return Err("HLS playlist contained no segments".into());
        }

        // Extract durations for seeking support.
        let segment_durations: Vec<f64> = segment_urls
            .iter()
            .map(|r| r.duration.unwrap_or(0.0))
            .collect();
        let has_durations = segment_durations.iter().any(|&d| d > 0.0);

        let all_segments = segment_urls.clone();

        // ── Bootstrap: fetch init segment + first batch synchronously ──
        let mut initial_buf = Vec::with_capacity(512 * 1024);
        let mut cached_map_data = None;

        if let Some(map_res) = &map_url {
            let resolved = resolve_resource_static(map_res, &cipher_manager, &player_url)?;
            let mut map_data = Vec::new();
            fetch_segment_into(&client, &resolved, &mut map_data)?;
            initial_buf.extend_from_slice(&map_data);
            cached_map_data = Some(map_data);
        }

        // Fetch first batch into the active buffer so playback starts instantly.
        let first_batch_count = PREFETCH_SEGMENTS.min(segment_urls.len());
        let mut pending = segment_urls;
        let first_batch: Vec<Resource> = pending.drain(..first_batch_count).collect();

        /*
        tracing::debug!(
            "HLS: bootstrap fetching {} segments (0 → {})",
            first_batch.len(),
            first_batch.len()
        );
        */

        for res in &first_batch {
            let resolved = resolve_resource_static(res, &cipher_manager, &player_url)?;
            fetch_and_demux_into(&client, &resolved, &mut initial_buf)?;
        }

        let current_segment_index = first_batch.len();

        // ── Set up shared state and spawn background thread ──
        let shared_state = SharedState {
            next_buf: Vec::with_capacity(512 * 1024),
            need_data: true, // start filling immediately
            pending,
            current_segment_index,
            command: PrefetchCommand::Continue,
            seek_done: false,
            eos: false,
        };

        let shared = Arc::new((Mutex::new(shared_state), Condvar::new()));
        let shared_bg = Arc::clone(&shared);

        // Clone what the background thread needs.
        let bg_client = client;
        let bg_cipher = cipher_manager;
        let bg_player_url = player_url;
        let bg_cached_map = cached_map_data;
        let bg_all_segments = all_segments.clone();

        let bg_thread = std::thread::Builder::new()
            .name("hls-prefetch".into())
            .spawn(move || {
                prefetch_loop(
                    shared_bg,
                    bg_client,
                    bg_cipher,
                    bg_player_url,
                    bg_cached_map,
                    bg_all_segments,
                );
            })
            .expect("failed to spawn HLS prefetch thread");

        Ok(Self {
            buf: initial_buf,
            pos: 0,
            shared,
            bg_thread: Some(bg_thread),
            all_segments,
            segment_durations,
            has_durations,
        })
    }

    /// Seek to a position in milliseconds by skipping segments.
    fn seek_to_ms(&mut self, position_ms: u64) -> io::Result<u64> {
        if !self.has_durations {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "HLS streams without segment durations are not seekable",
            ));
        }

        let target_secs = position_ms as f64 / 1000.0;
        let mut elapsed = 0.0;
        let mut target_index = 0;

        for (i, &dur) in self.segment_durations.iter().enumerate() {
            if elapsed + dur <= target_secs {
                elapsed += dur;
                target_index = i + 1;
            } else {
                break;
            }
        }

        if target_index >= self.all_segments.len() {
            target_index = self.all_segments.len().saturating_sub(1);
        }

        tracing::debug!(
            "HLS seek to {}ms -> segment {} (elapsed {:.1}s)",
            position_ms,
            target_index,
            elapsed
        );

        // Clear active buffer.
        self.buf.clear();
        self.pos = 0;

        // Tell background thread to seek.
        {
            let (lock, cvar) = &*self.shared;
            let mut state = lock.lock().unwrap();
            state.command = PrefetchCommand::Seek(target_index);
            state.need_data = true;
            state.seek_done = false;
            cvar.notify_one();

            // Wait for the background thread to confirm seek is complete.
            while !state.seek_done {
                state = cvar.wait(state).unwrap();
            }
            state.seek_done = false;

            // Swap in whatever the background thread prepared.
            std::mem::swap(&mut self.buf, &mut state.next_buf);
            state.next_buf.clear();
            state.need_data = true;
            cvar.notify_one();
        }

        Ok(0)
    }
}

impl Read for HlsReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        // If we have data in the active buffer, serve it immediately.
        if self.pos < self.buf.len() {
            // If we're running low, wake the background thread early.
            let remaining = self.buf.len() - self.pos;
            if remaining <= LOW_WATER_BYTES {
                let (lock, cvar) = &*self.shared;
                if let Ok(mut state) = lock.try_lock() {
                    if !state.need_data && !state.eos {
                        state.need_data = true;
                        cvar.notify_one();
                    }
                }
            }

            let n = out.len().min(remaining);
            out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
            self.pos += n;
            return Ok(n);
        }

        // Active buffer exhausted — swap with the pre-filled next buffer.
        let (lock, cvar) = &*self.shared;
        let mut state = lock.lock().unwrap();

        // If the background thread hasn't finished yet, wait for it.
        // But first signal that we need data if not already signalled.
        if !state.need_data && state.next_buf.is_empty() && !state.eos {
            state.need_data = true;
            cvar.notify_one();
        }

        while state.next_buf.is_empty() && !state.eos {
            state = cvar.wait(state).unwrap();
        }

        if state.next_buf.is_empty() && state.eos {
            return Ok(0); // End of stream
        }

        // Instant swap: move pre-filled buffer into active position.
        self.buf.clear();
        self.pos = 0;
        std::mem::swap(&mut self.buf, &mut state.next_buf);
        state.next_buf.clear();

        // Signal background thread to start filling the next buffer.
        state.need_data = true;
        cvar.notify_one();
        drop(state);

        // Now serve from the freshly-swapped buffer.
        let available = &self.buf[self.pos..];
        let n = out.len().min(available.len());
        out[..n].copy_from_slice(&available[..n]);
        self.pos += n;
        Ok(n)
    }
}

impl Seek for HlsReader {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        match pos {
            SeekFrom::Start(ms) => self.seek_to_ms(ms),
            _ => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "HLS seek only supports SeekFrom::Start (milliseconds)",
            )),
        }
    }
}

impl MediaSource for HlsReader {
    fn is_seekable(&self) -> bool {
        self.has_durations
    }
    fn byte_len(&self) -> Option<u64> {
        None
    }
}

fn prefetch_loop(
    shared: Arc<(Mutex<SharedState>, Condvar)>,
    client: reqwest::blocking::Client,
    cipher_manager: Option<Arc<YouTubeCipherManager>>,
    player_url: Option<String>,
    cached_map_data: Option<Vec<u8>>,
    all_segments: Vec<Resource>,
) {
    let (lock, cvar) = &*shared;

    loop {
        // Wait until the reader signals it needs data.
        let mut state = lock.lock().unwrap();
        while !state.need_data {
            state = cvar.wait(state).unwrap();
        }

        // Check for commands.
        match std::mem::replace(&mut state.command, PrefetchCommand::Continue) {
            PrefetchCommand::Stop => {
                return;
            }

            PrefetchCommand::Seek(target_index) => {
                // Reset for seek.
                state.next_buf.clear();
                state.eos = false;
                state.current_segment_index = target_index;
                state.pending = all_segments[target_index..].to_vec();

                // Re-use cached map data if available.
                if let Some(map_data) = &cached_map_data {
                    state.next_buf.extend_from_slice(map_data);
                }

                // Fetch JUST ONE segment to start playback ASAP (minimal latency).
                // The remaining segments will be fetched in the next loop iteration.
                let count = if !state.pending.is_empty() { 1 } else { 0 };
                let batch: Vec<Resource> = state.pending.drain(..count).collect();

                // Drop the lock while fetching (network I/O).
                drop(state);

                let mut tmp_buf = Vec::with_capacity(256 * 1024);
                for res in &batch {
                    if let Ok(resolved) =
                        resolve_resource_static(res, &cipher_manager, &player_url)
                    {
                        if let Err(e) = fetch_and_demux_into(&client, &resolved, &mut tmp_buf) {
                            tracing::warn!("HLS prefetch: segment fetch error during seek: {}", e);
                        }
                    }
                }

                // Re-acquire lock and store data.
                let mut state = lock.lock().unwrap();
                state.next_buf.extend_from_slice(&tmp_buf);
                state.current_segment_index += batch.len();
                state.need_data = false;
                state.seek_done = true;
                state.eos = state.pending.is_empty();
                cvar.notify_one();
                continue;
            }

            PrefetchCommand::Continue => {
                // Normal prefetch path below.
            }
        }

        if state.pending.is_empty() {
            state.eos = true;
            state.need_data = false;
            cvar.notify_one();
            continue;
        }

        // Fetch the next batch of segments.
        let count = PREFETCH_SEGMENTS.min(state.pending.len());
        let batch: Vec<Resource> = state.pending.drain(..count).collect();
        let seg_idx = state.current_segment_index;

        // Drop lock while doing network I/O.
        drop(state);

        /*
        tracing::debug!(
            "HLS prefetch: fetching {} segments (index {} → {})",
            batch.len(),
            seg_idx,
            seg_idx + batch.len()
        );
        */

        let mut tmp_buf = Vec::with_capacity(256 * 1024);
        for res in &batch {
            // Check for abort (stop or seek) between segments.
            {
                let s = lock.lock().unwrap();
                match &s.command {
                    PrefetchCommand::Stop => return,
                    PrefetchCommand::Seek(_) => {
                        // A seek was requested while we were fetching. 
                        // Re-enter the loop to handle it.
                        let mut s = s;
                        s.need_data = true;
                        cvar.notify_one();
                        break;
                    }
                    PrefetchCommand::Continue => {}
                }
            }

            if let Ok(resolved) = resolve_resource_static(res, &cipher_manager, &player_url) {
                if let Err(e) = fetch_and_demux_into(&client, &resolved, &mut tmp_buf) {
                    tracing::warn!("HLS prefetch: segment fetch error: {}", e);
                }
            }
        }

        // Re-acquire lock and store the fetched data.
        let mut state = lock.lock().unwrap();
        // Only append if no seek was requested while we were fetching.
        if matches!(state.command, PrefetchCommand::Continue) {
            state.next_buf.extend_from_slice(&tmp_buf);
            state.current_segment_index = seg_idx + batch.len();
            state.eos = state.pending.is_empty();
        }
        state.need_data = false;
        cvar.notify_one();
    }
}

/// Resolve a resource's URL (YouTube cipher / n-token handling).
fn resolve_resource_static(
    res: &Resource,
    cipher_manager: &Option<Arc<YouTubeCipherManager>>,
    player_url: &Option<String>,
) -> Result<Resource, Box<dyn std::error::Error + Send + Sync>> {
    let mut resolved = res.clone();
    resolved.url = resolve_url_string(&res.url, cipher_manager, player_url)?;
    Ok(resolved)
}

/// Fetch a media segment and demux TS → ADTS if it appears to be MPEG-TS.
fn fetch_and_demux_into(
    client: &reqwest::blocking::Client,
    res: &Resource,
    out: &mut Vec<u8>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut raw = Vec::new();
    fetch_segment_into(client, res, &mut raw)?;

    let is_ts = raw.first() == Some(&0x47);
    if is_ts {
        let adts = extract_adts_from_ts(&raw);
        if !adts.is_empty() {
            // tracing::debug!("HLS: demuxed {} TS bytes → {} ADTS bytes", raw.len(), adts.len());
            out.extend_from_slice(&adts);
        } else {
            tracing::warn!("HLS: TS demux produced no output, using raw segment");
            out.extend_from_slice(&raw);
        }
    } else {
        out.extend_from_slice(&raw);
    }
    Ok(())
}
