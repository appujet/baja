use std::{
  io::{Read, Seek, SeekFrom},
  sync::{Arc, Condvar, Mutex},
  thread,
};

use symphonia::core::io::MediaSource;
use tracing::debug;

use crate::{
  audio::remote_reader::{BaseRemoteReader, create_client},
  common::types::AnyResult,
  configs::HttpProxyConfig,
  sources::youtube::hls::{
    fetcher::fetch_segment_into, resolver::resolve_playlist, ts_demux::extract_adts_from_ts,
    types::Resource,
  },
};

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36";
const PREFETCH_SEGMENTS: usize = 3;
const LOW_WATER_BYTES: usize = 128 * 1024;

/// Reader for progressive SoundCloud streams (MP3/AAC).
pub struct SoundCloudReader {
  inner: BaseRemoteReader,
}

impl SoundCloudReader {
  pub fn new(
    url: &str,
    local_addr: Option<std::net::IpAddr>,
    proxy: Option<HttpProxyConfig>,
  ) -> AnyResult<Self> {
    let client = create_client(USER_AGENT.to_string(), local_addr, proxy, None)?;
    let inner = BaseRemoteReader::new(client, url)?;
    Ok(Self { inner })
  }
}

impl Read for SoundCloudReader {
  fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
    self.inner.read(buf)
  }
}

impl Seek for SoundCloudReader {
  fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
    self.inner.seek(pos)
  }
}

impl MediaSource for SoundCloudReader {
  fn is_seekable(&self) -> bool {
    self.inner.is_seekable()
  }

  fn byte_len(&self) -> Option<u64> {
    self.inner.byte_len()
  }
}

#[derive(Debug, Clone)]
enum PrefetchCommand {
  Continue,
  Seek(usize), // Segment index
  Stop,
}

struct SharedState {
  next_buf: Vec<u8>,
  need_data: bool,
  pending: Vec<Resource>,
  current_segment_index: usize,
  command: PrefetchCommand,
  seek_done: bool,
  eos: bool,
}

/// Specialized HLS reader for SoundCloud.
/// Implements bitrate-accurate byte seeking to satisfy symphonia's decoder.
pub struct SoundCloudHlsReader {
  /// Active buffer being consumed by `read()`.
  buf: Vec<u8>,
  /// Read cursor inside `buf`.
  pos: usize,
  /// Total bytes read globally across all buffers (virtual byte offset).
  total_bytes_read: u64,
  /// Shared mutable state protected by a mutex + condvar.
  shared: Arc<(Mutex<SharedState>, Condvar)>,
  /// Handle to the background prefetch thread.
  bg_thread: Option<thread::JoinHandle<()>>,
  /// All segments (kept for seeking).
  all_segments: Vec<Resource>,
  /// Segment durations in seconds.
  segment_durations: Vec<f64>,
  /// Bitrate in bytes per second (not bits per second).
  byte_rate: u64,
}

impl SoundCloudHlsReader {
  pub fn new(
    manifest_url: &str,
    bitrate_bps: u64,
    local_addr: Option<std::net::IpAddr>,
    proxy: Option<HttpProxyConfig>,
  ) -> AnyResult<Self> {
    let handle = tokio::runtime::Handle::current();
    let client = create_client(USER_AGENT.to_string(), local_addr, proxy, None)?;
    let (segment_urls, _map_url) = handle.block_on(resolve_playlist(&client, manifest_url))?;
    if segment_urls.is_empty() {
      return Err("SoundCloud HLS: playlist contained no segments".into());
    }

    let segment_durations: Vec<f64> = segment_urls
      .iter()
      .map(|r| r.duration.unwrap_or(0.0))
      .collect();
    let all_segments = segment_urls.clone();

    // SoundCloud bitrate is in bits per second. We need bytes per second for seeking.
    let byte_rate = bitrate_bps / 8;

    // ── Bootstrap: fetch first batch into active buffer ──
    let mut initial_buf = Vec::with_capacity(512 * 1024);
    let first_batch_count = PREFETCH_SEGMENTS.min(segment_urls.len());
    let mut pending = segment_urls;
    let first_batch: Vec<Resource> = pending.drain(..first_batch_count).collect();

    for res in &first_batch {
      let _ = handle.block_on(fetch_and_demux_into(&client, res, &mut initial_buf));
    }

    debug!(
      "SoundCloud HLS init: {} segments, bitrate={} bps ({} B/s)",
      all_segments.len(),
      bitrate_bps,
      byte_rate
    );

    let shared_state = SharedState {
      next_buf: Vec::with_capacity(512 * 1024),
      need_data: true,
      pending,
      current_segment_index: first_batch.len(),
      command: PrefetchCommand::Continue,
      seek_done: false,
      eos: false,
    };

    let shared = Arc::new((Mutex::new(shared_state), Condvar::new()));
    let shared_bg = Arc::clone(&shared);
    let bg_client = client;
    let bg_all = all_segments.clone();

    let handle_clone = handle.clone();
    let bg_thread = thread::Builder::new()
      .name("sc-hls-prefetch".into())
      .spawn(move || {
        prefetch_loop(shared_bg, bg_client, bg_all, handle_clone);
      })?;

    Ok(Self {
      buf: initial_buf,
      pos: 0,
      total_bytes_read: 0,
      shared,
      bg_thread: Some(bg_thread),
      all_segments,
      segment_durations,
      byte_rate,
    })
  }

  fn seek_to_byte(&mut self, target_byte: u64) -> std::io::Result<u64> {
    let current_byte = self.total_bytes_read;
    let diff = (target_byte as i64) - (current_byte as i64);

    // Gapless optimization: seek within the already demuxed active buffer.
    let buf_len = self.buf.len() as i64;
    let current_pos_in_buf = self.pos as i64;
    let new_pos_in_buf = current_pos_in_buf + diff;

    if new_pos_in_buf >= 0 && new_pos_in_buf <= buf_len {
      debug!(
        "SoundCloud HLS gapless seek (internal buffer): {} -> {} (pos {} -> {})",
        current_byte, target_byte, self.pos, new_pos_in_buf
      );
      self.pos = new_pos_in_buf as usize;
      self.total_bytes_read = target_byte;
      return Ok(target_byte);
    }

    // Hard seek: map byte offset to segment index.
    let target_secs = target_byte as f64 / self.byte_rate as f64;
    let mut segment_start_secs = 0.0;
    let mut target_index = 0;

    for (i, &dur) in self.segment_durations.iter().enumerate() {
      if segment_start_secs + dur <= target_secs {
        segment_start_secs += dur;
        target_index = i + 1;
      } else {
        break;
      }
    }

    if target_index >= self.all_segments.len() {
      target_index = self.all_segments.len().saturating_sub(1);
    }

    // Calculate virtual skip within the physical segment.
    let segment_start_byte = (segment_start_secs * self.byte_rate as f64) as u64;
    let skip_in_segment = target_byte.saturating_sub(segment_start_byte);

    debug!(
      "SoundCloud HLS hard seek: target {} -> segment {} (starts at {:.1}s, segment-relative skip={} bytes)",
      target_byte, target_index, segment_start_secs, skip_in_segment
    );

    // Clear active buffer
    self.buf.clear();
    self.pos = 0;
    self.total_bytes_read = target_byte;

    // Signal prefetcher
    {
      let (lock, cvar) = &*self.shared;
      let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
      state.command = PrefetchCommand::Seek(target_index);
      state.need_data = true;
      state.seek_done = false;
      cvar.notify_one();

      while !state.seek_done {
        state = cvar.wait(state).unwrap();
      }
      state.seek_done = false;

      // Swap in the seek target segment
      std::mem::swap(&mut self.buf, &mut state.next_buf);
      state.next_buf.clear();
      debug!(
        "SoundCloud HLS swapped buffers after hard seek. Buffer len: {}",
        self.buf.len()
      );

      // Align the buffer position to match the requested virtual byte.
      self.pos = (skip_in_segment as usize).min(self.buf.len());
      if self.pos > 0 || skip_in_segment > 0 {
        debug!(
          "SoundCloud HLS aligned buffer position to offset {} (skip_in_segment={})",
          self.pos, skip_in_segment
        );
      }

      state.need_data = true;
      cvar.notify_one();
    }

    Ok(target_byte)
  }
}

impl Read for SoundCloudHlsReader {
  fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
    if self.pos < self.buf.len() {
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
      self.total_bytes_read += n as u64;
      return Ok(n);
    }

    let (lock, cvar) = &*self.shared;
    let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());

    if !state.need_data && state.next_buf.is_empty() && !state.eos {
      state.need_data = true;
      cvar.notify_one();
    }

    while state.next_buf.is_empty() && !state.eos {
      state = cvar.wait(state).unwrap();
    }

    if state.next_buf.is_empty() && state.eos {
      return Ok(0);
    }

    // Swap next buffer into active position
    let next_len = state.next_buf.len();
    self.buf.clear();
    self.pos = 0;
    std::mem::swap(&mut self.buf, &mut state.next_buf);
    state.next_buf.clear();
    debug!(
      "SoundCloud HLS buffer swap: replaced active with next_buf ({} bytes)",
      next_len
    );

    state.need_data = true;
    cvar.notify_one();
    drop(state);

    self.read(out)
  }
}

impl Seek for SoundCloudHlsReader {
  fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
    match pos {
      SeekFrom::Start(n) => self.seek_to_byte(n),
      SeekFrom::Current(delta) => {
        let target = self.total_bytes_read.saturating_add_signed(delta);
        self.seek_to_byte(target)
      }
      SeekFrom::End(_) => {
        let total = self.byte_len().unwrap_or(0);
        self.seek_to_byte(total)
      }
    }
  }
}

impl MediaSource for SoundCloudHlsReader {
  fn is_seekable(&self) -> bool {
    true
  }
  fn byte_len(&self) -> Option<u64> {
    let total_dur: f64 = self.segment_durations.iter().sum();
    Some((total_dur * self.byte_rate as f64) as u64)
  }
}

impl Drop for SoundCloudHlsReader {
  fn drop(&mut self) {
    let (lock, cvar) = &*self.shared;
    let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
    state.command = PrefetchCommand::Stop;
    state.need_data = true;
    cvar.notify_one();
    if let Some(handle) = self.bg_thread.take() {
      let _ = handle.join();
    }
  }
}

fn prefetch_loop(
  shared: Arc<(Mutex<SharedState>, Condvar)>,
  client: reqwest::Client,
  all_segments: Vec<Resource>,
  handle: tokio::runtime::Handle,
) {
  let (lock, cvar) = &*shared;

  loop {
    let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
    while !state.need_data {
      state = cvar.wait(state).unwrap();
    }

    match std::mem::replace(&mut state.command, PrefetchCommand::Continue) {
      PrefetchCommand::Stop => return,
      PrefetchCommand::Seek(target_index) => {
        state.next_buf.clear();
        state.eos = false;
        state.current_segment_index = target_index;
        state.pending = all_segments[target_index..].to_vec();

        let count = if !state.pending.is_empty() { 1 } else { 0 };
        let batch: Vec<Resource> = state.pending.drain(..count).collect();
        drop(state);

        let mut tmp_buf = Vec::new();
        for res in &batch {
          debug!(
            "SoundCloud HLS prefetcher: fetching seek target segment {}",
            target_index
          );
          let _ = handle.block_on(fetch_and_demux_into(&client, res, &mut tmp_buf));
        }
        debug!(
          "SoundCloud HLS prefetcher: seek target fetched ({} bytes)",
          tmp_buf.len()
        );

        let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
        state.next_buf.extend_from_slice(&tmp_buf);
        state.current_segment_index += batch.len();
        state.need_data = false;
        state.seek_done = true;
        state.eos = state.pending.is_empty();
        cvar.notify_one();
        continue;
      }
      PrefetchCommand::Continue => {}
    }

    if state.pending.is_empty() {
      state.eos = true;
      state.need_data = false;
      cvar.notify_one();
      continue;
    }

    let count = PREFETCH_SEGMENTS.min(state.pending.len());
    let batch: Vec<Resource> = state.pending.drain(..count).collect();
    let current_idx = state.current_segment_index;
    drop(state);

    let mut tmp_buf = Vec::with_capacity(256 * 1024);
    for res in &batch {
      {
        let s = lock.lock().unwrap_or_else(|e| e.into_inner());
        if !matches!(s.command, PrefetchCommand::Continue) {
          break;
        }
      }
      let _ = handle.block_on(fetch_and_demux_into(&client, res, &mut tmp_buf));
    }

    let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
    if !matches!(state.command, PrefetchCommand::Continue) {
      // Re-enter the loop to immediately handle the new command (e.g. Seek)
      // without resetting need_data to false, which would cause a deadlock
      continue;
    }

    state.next_buf.extend_from_slice(&tmp_buf);
    state.current_segment_index = current_idx + batch.len();
    state.eos = state.pending.is_empty();
    state.need_data = false;
    cvar.notify_one();
  }
}

async fn fetch_and_demux_into(
  client: &reqwest::Client,
  res: &Resource,
  out: &mut Vec<u8>,
) -> AnyResult<()> {
  let mut raw = Vec::new();
  fetch_segment_into(client, res, &mut raw).await?;

  // If segment is MPEG-TS (starts with sync byte 0x47), demux to ADTS/MP3
  if raw.first() == Some(&0x47) {
    let adts = extract_adts_from_ts(&raw);
    if !adts.is_empty() {
      out.extend_from_slice(&adts);
    } else {
      out.extend_from_slice(&raw);
    }
  } else {
    out.extend_from_slice(&raw);
  }
  Ok(())
}
