use crate::api::tracks::TrackInfo;

pub fn decode_song_batch(buf: &[u8]) -> Vec<(String, TrackInfo)> {
  let mut songs = Vec::new();
  let mut reader = ProtoReader::new(buf);
  while reader.has_more() {
    let tag = match reader.read_uint32() { Some(t) => t, None => break };
    let field_no = tag >> 3;
    let wire_type = tag & 7;

    if field_no == 2 {
      if let Some(len) = reader.read_uint32() {
        let end = reader.pos + len as usize;
        let mut key = String::new();
        let mut song = None;
        while reader.pos < end {
          let map_tag = match reader.read_uint32() { Some(t) => t, None => break };
          match map_tag >> 3 {
            1 => key = reader.read_string().unwrap_or_default(),
            2 => {
              if let Some(song_len) = reader.read_uint32() {
                song = decode_song(reader.read_slice(song_len as usize));
              }
            }
            _ => reader.skip_type(map_tag & 7),
          }
        }
        if !key.is_empty() {
          if let Some(s) = song {
            songs.push((key, s));
          }
        }
      }
    } else {
      reader.skip_type(wire_type);
    }
  }
  songs
}

fn decode_song(buf: &[u8]) -> Option<TrackInfo> {
  let mut reader = ProtoReader::new(buf);
  let mut id = String::new();
  let mut title = String::new();
  let mut artist = String::new();
  let mut duration = 0.0f32;
  let mut cover_art = String::new();

  while reader.has_more() {
    let tag = match reader.read_uint32() { Some(t) => t, None => break };
    let field_no = tag >> 3;
    match field_no {
      1 => id = reader.read_string().unwrap_or_default(),
      2 => title = reader.read_string().unwrap_or_default(),
      5 => artist = reader.read_string().unwrap_or_default(),
      9 => duration = reader.read_float().unwrap_or(0.0),
      10 => cover_art = reader.read_string().unwrap_or_default(),
      _ => reader.skip_type(tag & 7),
    }
  }

  if id.is_empty() || title.is_empty() { return None; }

  let artwork_url = if !cover_art.is_empty() {
    Some(format!("https://artwork.anghcdn.co/?id={}&size=640", cover_art))
  } else {
    None
  };

  Some(TrackInfo {
    identifier: id.clone(),
    is_seekable: true,
    author: if artist.is_empty() { "Unknown Artist".to_string() } else { artist },
    length: (duration * 1000.0).round() as u64,
    is_stream: false,
    position: 0,
    title,
    uri: Some(format!("https://play.anghami.com/song/{}", id)),
    artwork_url,
    isrc: None,
    source_name: "anghami".to_string(),
  })
}

struct ProtoReader<'a> {
  buf: &'a [u8],
  pos: usize,
}

impl<'a> ProtoReader<'a> {
  fn new(buf: &'a [u8]) -> Self {
    Self { buf, pos: 0 }
  }

  fn has_more(&self) -> bool {
    self.pos < self.buf.len()
  }

  fn read_uint32(&mut self) -> Option<u32> {
    let mut value = 0u32;
    let mut shift = 0;
    while self.pos < self.buf.len() {
      let b = self.buf[self.pos];
      self.pos += 1;
      value |= ((b & 127) as u32) << shift;
      if b < 128 { return Some(value); }
      shift += 7;
      if shift >= 35 { break; }
    }
    None
  }

  fn read_string(&mut self) -> Option<String> {
    let len = self.read_uint32()? as usize;
    if self.pos + len > self.buf.len() { return None; }
    let s = String::from_utf8_lossy(&self.buf[self.pos..self.pos+len]).to_string();
    self.pos += len;
    Some(s)
  }

  fn read_float(&mut self) -> Option<f32> {
    if self.pos + 4 > self.buf.len() { return None; }
    let mut b = [0u8; 4];
    b.copy_from_slice(&self.buf[self.pos..self.pos+4]);
    self.pos += 4;
    Some(f32::from_le_bytes(b))
  }

  fn read_slice(&mut self, len: usize) -> &[u8] {
    let end = std::cmp::min(self.pos + len, self.buf.len());
    let slice = &self.buf[self.pos..end];
    self.pos = end;
    slice
  }

  fn skip_type(&mut self, wire_type: u32) {
    match wire_type {
      0 => { self.read_uint32(); },
      1 => { self.pos += 8; },
      2 => { 
        if let Some(len) = self.read_uint32() {
          self.pos += len as usize;
        }
      },
      5 => { self.pos += 4; },
      _ => {}
    }
  }
}
