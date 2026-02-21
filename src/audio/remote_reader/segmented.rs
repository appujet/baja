use std::{
  collections::HashMap,
  io::{Read, Seek, SeekFrom},
  sync::{Arc, Condvar, Mutex},
  thread,
};

use symphonia::core::io::MediaSource;
use tracing::{debug, info, warn};

use crate::common::types::AnyResult;

const CHUNK_SIZE: usize = 512 * 1024; // 512KB chunks
const PREFETCH_CHUNKS: usize = 32; // 16MB prefetch window
const MAX_CONCURRENT_FETCHES: usize = 6;

#[derive(Clone)]
enum ChunkState {
  Empty,
  Downloading,
  Ready(Arc<Vec<u8>>),
}

struct ReaderState {
  chunks: HashMap<usize, ChunkState>,
  current_pos: u64,
  total_len: u64,
  is_terminated: bool,
}

pub struct SegmentedRemoteReader {
  pos: u64,
  len: u64,
  content_type: Option<String>,
  shared: Arc<(Mutex<ReaderState>, Condvar)>,
}

impl SegmentedRemoteReader {
  pub fn new(client: reqwest::blocking::Client, url: &str) -> AnyResult<Self> {
    // 1. Initial request to get length and first chunk
    let response = Self::fetch_range(&client, url, 0, CHUNK_SIZE as u64)?;

    let len = response
      .headers()
      .get(reqwest::header::CONTENT_RANGE)
      .and_then(|v| v.to_str().ok())
      .and_then(|v| v.split('/').last())
      .and_then(|v| v.parse::<u64>().ok())
      .or_else(|| response.content_length())
      .ok_or("Failed to get content length")?;

    let content_type = response
      .headers()
      .get(reqwest::header::CONTENT_TYPE)
      .and_then(|v| v.to_str().ok())
      .map(str::to_string);

    info!(
      "Opened SegmentedRemoteReader: {} (len={}, type={:?})",
      url, len, content_type
    );

    let mut first_chunk = Vec::with_capacity(CHUNK_SIZE);
    let mut r = response;
    r.read_to_end(&mut first_chunk)?;

    let mut chunks = HashMap::new();
    chunks.insert(0, ChunkState::Ready(Arc::new(first_chunk)));

    let shared = Arc::new((
      Mutex::new(ReaderState {
        chunks,
        current_pos: 0,
        total_len: len,
        is_terminated: false,
      }),
      Condvar::new(),
    ));

    // 2. Spawn background workers
    for i in 0..MAX_CONCURRENT_FETCHES {
      let shared_clone = shared.clone();
      let client_clone = client.clone();
      let url_str = url.to_string();
      thread::Builder::new()
        .name(format!("segmented-fetch-{}", i))
        .spawn(move || {
          fetch_worker(shared_clone, client_clone, url_str);
        })?;
    }

    Ok(Self {
      pos: 0,
      len,
      content_type,
      shared,
    })
  }

  fn fetch_range(
    client: &reqwest::blocking::Client,
    url: &str,
    offset: u64,
    size: u64,
  ) -> AnyResult<reqwest::blocking::Response> {
    let range = format!("bytes={}-{}", offset, offset + size - 1);
    let res = client
      .get(url)
      .header("Range", range)
      .header("Accept", "*/*")
      .header("Connection", "keep-alive")
      .send()?;

    if !res.status().is_success() && res.status() != reqwest::StatusCode::PARTIAL_CONTENT {
      return Err(format!("Fetch failed: {}", res.status()).into());
    }
    Ok(res)
  }

  pub fn content_type(&self) -> Option<String> {
    self.content_type.clone()
  }
}

fn fetch_worker(
  shared: Arc<(Mutex<ReaderState>, Condvar)>,
  client: reqwest::blocking::Client,
  url: String,
) {
  let (lock, cvar) = &*shared;

  loop {
    let mut target_chunk_idx = None;

    {
      let mut state = lock.lock().unwrap();
      if state.is_terminated {
        break;
      }

      let current_chunk_idx = (state.current_pos / CHUNK_SIZE as u64) as usize;

      // 1. High priority: check if the chunk we're currently at is needed
      if let Some(ChunkState::Empty) = state.chunks.get(&current_chunk_idx) {
        state
          .chunks
          .insert(current_chunk_idx, ChunkState::Downloading);
        target_chunk_idx = Some(current_chunk_idx);
      }
      // 2. Next: fill the rest of the prefetch window as fast as possible
      else {
        for i in 0..PREFETCH_CHUNKS {
          let idx = current_chunk_idx + i;
          if (idx * CHUNK_SIZE) as u64 >= state.total_len {
            break;
          }

          let entry = state.chunks.entry(idx).or_insert(ChunkState::Empty);
          if matches!(entry, ChunkState::Empty) {
            *entry = ChunkState::Downloading;
            target_chunk_idx = Some(idx);
            break;
          }
        }
      }

      if target_chunk_idx.is_none() {
        // Nothing to do, wait for position change or timeout (short wait for fast reaction)
        let _ = cvar
          .wait_timeout(state, std::time::Duration::from_millis(50))
          .unwrap();
        continue;
      }
    }

    if let Some(idx) = target_chunk_idx {
      let offset = (idx * CHUNK_SIZE) as u64;
      let stream_len = lock.lock().unwrap().total_len;
      let size = CHUNK_SIZE.min((stream_len - offset) as usize);

      match SegmentedRemoteReader::fetch_range(&client, &url, offset, size as u64) {
        Ok(mut res) => {
          let mut data = Vec::with_capacity(size);
          if let Ok(_) = res.read_to_end(&mut data) {
            let mut state = lock.lock().unwrap();
            let actual_len = data.len();
            state.chunks.insert(idx, ChunkState::Ready(Arc::new(data)));
            debug!(
              "SegmentedRemoteReader worker filled chunk {} ({} bytes)",
              idx, actual_len
            );
            cvar.notify_all();
          } else {
            let mut state = lock.lock().unwrap();
            state.chunks.insert(idx, ChunkState::Empty);
            cvar.notify_all();
          }
        }
        Err(e) => {
          warn!("SegmentedRemoteReader fetch error for chunk {}: {}", idx, e);
          let mut state = lock.lock().unwrap();
          state.chunks.insert(idx, ChunkState::Empty);
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
    let mut state = lock.lock().unwrap();

    // Sync our position with the background workers
    state.current_pos = self.pos;

    loop {
      let chunk_idx = (self.pos / CHUNK_SIZE as u64) as usize;
      let offset_in_chunk = (self.pos % CHUNK_SIZE as u64) as usize;

      match state.chunks.get(&chunk_idx) {
        Some(ChunkState::Ready(data)) => {
          let available = data.len().saturating_sub(offset_in_chunk);
          if available == 0 {
            if self.pos >= self.len {
              return Ok(0);
            }
            // Move to the next chunk boundary
            self.pos = ((chunk_idx + 1) * CHUNK_SIZE) as u64;
            state.current_pos = self.pos;
            continue;
          }

          let n = buf.len().min(available);
          buf[..n].copy_from_slice(&data[offset_in_chunk..offset_in_chunk + n]);
          self.pos += n as u64;
          state.current_pos = self.pos;

          // Proactively clean up old chunks to save memory
          if chunk_idx > 8 {
            state.chunks.retain(|&idx, _| idx >= chunk_idx - 4);
          }
          return Ok(n);
        }
        Some(ChunkState::Downloading) | Some(ChunkState::Empty) => {
          if let Some(ChunkState::Empty) = state.chunks.get(&chunk_idx) {
            state.chunks.insert(chunk_idx, ChunkState::Downloading);
          }
          cvar.notify_all();

          let (new_state, result) = cvar
            .wait_timeout(state, std::time::Duration::from_millis(1000))
            .unwrap();
          state = new_state;

          if result.timed_out() {
            debug!(
              "SegmentedRemoteReader read wait timed out for chunk {}",
              chunk_idx
            );
          }
        }
        None => {
          state.chunks.insert(chunk_idx, ChunkState::Empty);
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

    if new_pos > self.len {
      self.pos = self.len;
    } else {
      self.pos = new_pos;
    }

    let (lock, cvar) = &*self.shared;
    let mut state = lock.lock().unwrap();
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
    let mut state = lock.lock().unwrap();
    state.is_terminated = true;
    cvar.notify_all();
  }
}
