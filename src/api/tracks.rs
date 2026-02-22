use std::io::{Cursor, Read, Write};

use base64::prelude::*;
use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use serde::{Deserialize, Serialize};

use crate::common::Severity;

/// A single audio track with encoded data and metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
  /// Base64-encoded track data.
  pub encoded: String,
  /// Track metadata.
  pub info: TrackInfo,
  /// Plugin-specific info. Always `{}` without plugins.
  #[serde(default)]
  pub plugin_info: serde_json::Value,
  /// User-provided data attached to the track.
  #[serde(default)]
  pub user_data: serde_json::Value,
}

impl Track {
  /// Create a new Track from info and encode it.
  pub fn new(info: TrackInfo) -> Self {
    let mut track = Self {
      encoded: String::new(),
      info,
      plugin_info: serde_json::json!({}),
      user_data: serde_json::json!({}),
    };
    track.encoded = track.encode();
    track
  }

  /// Encode the track into a Lavalink-compatible base64 string.
  pub fn encode(&self) -> String {
    let mut buf = Vec::new();
    // Version 3 (Lavalink v4 standard)
    buf.write_u8(3).unwrap();

    write_utf(&mut buf, &self.info.title);
    write_utf(&mut buf, &self.info.author);
    buf.write_u64::<BigEndian>(self.info.length).unwrap();
    write_utf(&mut buf, &self.info.identifier);
    buf
      .write_u8(if self.info.is_stream { 1 } else { 0 })
      .unwrap();

    write_opt_utf(&mut buf, self.info.uri.as_deref());
    write_opt_utf(&mut buf, self.info.artwork_url.as_deref());
    write_opt_utf(&mut buf, self.info.isrc.as_deref());
    write_utf(&mut buf, &self.info.source_name);

    buf.write_u64::<BigEndian>(self.info.position).unwrap();

    BASE64_STANDARD.encode(&buf)
  }

  /// Decode a Lavalink track from a base64 string.
  pub fn decode(encoded: &str) -> Option<Self> {
    let data = BASE64_STANDARD.decode(encoded).ok()?;
    let mut cursor = Cursor::new(data);

    let version = cursor.read_u8().ok()?;
    if version > 3 {
      return None; // Unknown version
    }

    let title = read_utf(&mut cursor)?;
    let author = read_utf(&mut cursor)?;
    let length = cursor.read_u64::<BigEndian>().ok()?;
    let identifier = read_utf(&mut cursor)?;
    let is_stream = cursor.read_u8().ok()? != 0;

    let uri = read_opt_utf(&mut cursor);
    let artwork_url = if version >= 3 {
      read_opt_utf(&mut cursor)
    } else {
      None
    };
    let isrc = if version >= 3 {
      read_opt_utf(&mut cursor)
    } else {
      None
    };
    let source_name = read_utf(&mut cursor)?;

    let position = if version >= 2 {
      cursor.read_u64::<BigEndian>().ok().unwrap_or(0)
    } else {
      0
    };

    Some(Self {
      encoded: encoded.to_string(),
      info: TrackInfo {
        identifier,
        is_seekable: !is_stream,
        author,
        length,
        is_stream,
        position,
        title,
        uri,
        artwork_url,
        isrc,
        source_name,
      },
      plugin_info: serde_json::json!({}),
      user_data: serde_json::json!({}),
    })
  }
}

fn write_utf(w: &mut Vec<u8>, s: &str) {
  let bytes = s.as_bytes();
  w.write_u16::<BigEndian>(bytes.len() as u16).unwrap();
  w.write_all(bytes).unwrap();
}

fn write_opt_utf(w: &mut Vec<u8>, s: Option<&str>) {
  match s {
    Some(s) => {
      w.write_u8(1).unwrap();
      write_utf(w, s);
    }
    None => {
      w.write_u8(0).unwrap();
    }
  }
}

fn read_utf<R: Read>(r: &mut R) -> Option<String> {
  let len = r.read_u16::<BigEndian>().ok()? as usize;
  let mut buf = vec![0u8; len];
  r.read_exact(&mut buf).ok()?;
  String::from_utf8(buf).ok()
}

fn read_opt_utf<R: Read>(r: &mut R) -> Option<String> {
  let present = r.read_u8().ok()? != 0;
  if present { read_utf(r) } else { None }
}

/// Metadata for an audio track.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackInfo {
  pub identifier: String,
  pub is_seekable: bool,
  pub author: String,
  /// Duration in milliseconds. 0 for streams.
  pub length: u64,
  pub is_stream: bool,
  /// Current playback position in milliseconds.
  pub position: u64,
  pub title: String,
  pub uri: Option<String>,
  pub artwork_url: Option<String>,
  pub isrc: Option<String>,
  pub source_name: String,
}

/// Result of a track load operation.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "loadType", content = "data", rename_all = "camelCase")]
pub enum LoadResult {
  /// A single track was loaded.
  Track(Track),
  /// A playlist was loaded.
  Playlist(PlaylistData),
  /// A search returned results.
  Search(Vec<Track>),
  /// No matches found.
  Empty {},
  /// An error occurred during loading.
  Error(LoadError),
}

/// Playlist data returned from a load operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistData {
  pub info: PlaylistInfo,
  pub plugin_info: serde_json::Value,
  pub tracks: Vec<Track>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextData {
  pub text: String,
  pub plugin: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
  pub tracks: Vec<Track>,
  pub albums: Vec<PlaylistData>,
  pub artists: Vec<PlaylistData>,
  pub playlists: Vec<PlaylistData>,
  pub texts: Vec<TextData>,
  pub plugin: serde_json::Value,
}

/// Playlist metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistInfo {
  pub name: String,
  /// Index of the selected track, or -1 if none.
  pub selected_track: i32,
}

/// Error from a failed track load.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadError {
  pub message: String,
  pub severity: Severity,
  pub cause: String,
}
