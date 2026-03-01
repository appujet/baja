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
    /// Plugin-specific info — free JSON object whose shape is defined by the plugin.
    #[serde(default = "default_json_object")]
    pub plugin_info: serde_json::Value,
    /// User-provided data attached to the track.
    #[serde(default = "default_json_object")]
    pub user_data: serde_json::Value,
}

fn default_json_object() -> serde_json::Value {
    serde_json::json!({})
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

    /// Encode the track into a base64 string.
    ///
    /// Binary format (Lavaplayer-compatible, version 3):
    ///   [u32 header: (payload_size) | (flags << 30)]
    ///     flags bit 0 = TRACK_INFO_VERSIONED (version byte present)
    ///   [u8  version = 3]
    ///   [utf title]
    ///   [utf author]
    ///   [u64 length ms]
    ///   [utf identifier]
    ///   [u8  is_stream: 0/1]
    ///   [opt_utf uri]         -- v2+
    ///   [opt_utf artwork_url] -- v3+
    ///   [opt_utf isrc]        -- v3+
    ///   [utf source_name]
    ///   [u64 position ms]
    pub fn encode(&self) -> String {
        let mut msg_buf = Vec::new();
        // Version byte (Lavaplayer currently uses version 3)
        msg_buf.write_u8(3).unwrap();

        write_utf(&mut msg_buf, &self.info.title);
        write_utf(&mut msg_buf, &self.info.author);
        msg_buf.write_u64::<BigEndian>(self.info.length).unwrap();
        write_utf(&mut msg_buf, &self.info.identifier);
        msg_buf
            .write_u8(if self.info.is_stream { 1 } else { 0 })
            .unwrap();

        // v2+: optional URI
        write_opt_utf(&mut msg_buf, self.info.uri.as_deref());
        // v3+: optional artwork URL
        write_opt_utf(&mut msg_buf, self.info.artwork_url.as_deref());
        // v3+: optional ISRC
        write_opt_utf(&mut msg_buf, self.info.isrc.as_deref());

        write_utf(&mut msg_buf, &self.info.source_name);
        msg_buf.write_u64::<BigEndian>(self.info.position).unwrap();

        // Header: low 30 bits = payload size, high 2 bits = flags.
        // Bit 30 (flags & 1 = 1) = TRACK_INFO_VERSIONED (version byte present).
        let mut final_buf = Vec::new();
        let size = msg_buf.len() as u32;
        let flags: u32 = 1; // TRACK_INFO_VERSIONED
        let header = size | (flags << 30);
        final_buf.write_u32::<BigEndian>(header).unwrap();
        final_buf.extend_from_slice(&msg_buf);

        BASE64_STANDARD.encode(&final_buf)
    }

    /// Decode a track from a base64 string.
    ///
    /// Supports Lavaplayer track format versions 1, 2, and 3.
    pub fn decode(encoded: &str) -> Option<Self> {
        let data = BASE64_STANDARD.decode(encoded).ok()?;
        if data.len() < 4 {
            return None;
        }

        let mut cursor = Cursor::new(data);
        let header = cursor.read_u32::<BigEndian>().ok()?;
        // High 2 bits = flags; low 30 bits = payload size (not needed during decode).
        let flags = (header >> 30) & 0x03;

        // Bit 0 of flags = TRACK_INFO_VERSIONED: version byte follows header.
        // If not set, assume version 1 (legacy format).
        let version = if (flags & 1) != 0 {
            cursor.read_u8().ok()?
        } else {
            1
        };

        if version > 3 {
            // Unknown future version — refuse to corrupt data
            return None;
        }

        let title = read_utf(&mut cursor)?;
        let author = read_utf(&mut cursor)?;
        let length = cursor.read_u64::<BigEndian>().ok()?;
        let identifier = read_utf(&mut cursor)?;
        let is_stream = cursor.read_u8().ok()? != 0;

        // v2+: optional URI
        let uri = if version >= 2 {
            read_opt_utf(&mut cursor)
        } else {
            None
        };

        // v3+: optional artwork URL and ISRC
        let (artwork_url, isrc) = if version >= 3 {
            (read_opt_utf(&mut cursor), read_opt_utf(&mut cursor))
        } else {
            (None, None)
        };

        let source_name = read_utf(&mut cursor)?;

        // Position is at the end; treat missing as 0.
        let position = cursor.read_u64::<BigEndian>().ok().unwrap_or(0);

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
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TrackInfo {
    pub identifier: String,
    pub is_seekable: bool,
    pub author: String,
    /// Duration in milliseconds. 0 for live streams.
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
    /// Human-readable error message.
    pub message: Option<String>,
    /// How severe the error is.
    pub severity: Severity,
    /// Exception class / short cause description.
    pub cause: String,
    /// Full stack trace, if available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause_stack_trace: Option<String>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_info() -> TrackInfo {
        TrackInfo {
            identifier: "dQw4w9WgXcQ".to_string(),
            is_seekable: true,
            author: "Rick Astley".to_string(),
            length: 212000,
            is_stream: false,
            position: 0,
            title: "Never Gonna Give You Up".to_string(),
            uri: Some("https://www.youtube.com/watch?v=dQw4w9WgXcQ".to_string()),
            artwork_url: Some("https://i.ytimg.com/vi/dQw4w9WgXcQ/maxresdefault.jpg".to_string()),
            isrc: Some("GBARL9300135".to_string()),
            source_name: "youtube".to_string(),
        }
    }

    #[test]
    fn test_encode_decode_roundtrip() {
        let track = Track::new(sample_info());
        let decoded = Track::decode(&track.encoded).expect("decode should succeed");

        assert_eq!(decoded.info.identifier, "dQw4w9WgXcQ");
        assert_eq!(decoded.info.title, "Never Gonna Give You Up");
        assert_eq!(decoded.info.author, "Rick Astley");
        assert_eq!(decoded.info.length, 212000);
        assert_eq!(decoded.info.is_stream, false);
        assert_eq!(decoded.info.is_seekable, true);
        assert_eq!(decoded.info.position, 0);
        assert_eq!(
            decoded.info.uri.as_deref(),
            Some("https://www.youtube.com/watch?v=dQw4w9WgXcQ")
        );
        assert_eq!(
            decoded.info.artwork_url.as_deref(),
            Some("https://i.ytimg.com/vi/dQw4w9WgXcQ/maxresdefault.jpg")
        );
        assert_eq!(decoded.info.isrc.as_deref(), Some("GBARL9300135"));
        assert_eq!(decoded.info.source_name, "youtube");
    }

    #[test]
    fn test_encode_decode_stream() {
        let mut info = sample_info();
        info.is_stream = true;
        info.is_seekable = false;
        info.length = 0;
        info.uri = None;
        info.artwork_url = None;
        info.isrc = None;

        let track = Track::new(info);
        let decoded = Track::decode(&track.encoded).expect("decode should succeed");

        assert_eq!(decoded.info.is_stream, true);
        assert_eq!(decoded.info.is_seekable, false);
        assert_eq!(decoded.info.length, 0);
        assert_eq!(decoded.info.uri, None);
    }

    #[test]
    fn test_encode_decode_none_optionals() {
        let mut info = sample_info();
        info.uri = None;
        info.artwork_url = None;
        info.isrc = None;

        let track = Track::new(info);
        let decoded = Track::decode(&track.encoded).expect("decode should succeed");

        assert_eq!(decoded.info.uri, None);
        assert_eq!(decoded.info.artwork_url, None);
        assert_eq!(decoded.info.isrc, None);
    }

    #[test]
    fn test_decode_invalid_base64_returns_none() {
        assert!(Track::decode("not_valid_base64!!!").is_none());
    }

    #[test]
    fn test_decode_too_short_returns_none() {
        let short = BASE64_STANDARD.encode(&[1u8, 2u8, 3u8]);
        assert!(Track::decode(&short).is_none());
    }

    #[test]
    fn test_plugin_info_defaults_to_empty_object() {
        let track = Track::new(sample_info());
        assert_eq!(track.plugin_info, serde_json::json!({}));
        assert_eq!(track.user_data, serde_json::json!({}));
    }

    #[test]
    fn test_track_serializes_camelcase() {
        let track = Track::new(sample_info());
        let json = serde_json::to_value(&track).unwrap();

        assert!(json.get("pluginInfo").is_some(), "expected pluginInfo key");
        assert!(json.get("userData").is_some(), "expected userData key");
        assert!(json.get("info").is_some(), "expected info key");

        let info = &json["info"];
        assert!(info.get("isSeekable").is_some());
        assert!(info.get("isStream").is_some());
        assert!(info.get("artworkUrl").is_some());
        assert!(info.get("sourceName").is_some());
    }

    /// Verify that the encoded header byte has TRACK_INFO_VERSIONED flag set
    /// and the version byte is 3, matching Lavaplayer's format.
    #[test]
    fn test_header_format_matches_lavaplayer() {
        let track = Track::new(sample_info());
        let raw = BASE64_STANDARD.decode(&track.encoded).unwrap();

        let header = u32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]);
        let flags = (header >> 30) & 0x03;

        // Bit 0 must be set (TRACK_INFO_VERSIONED)
        assert_eq!(flags & 1, 1, "TRACK_INFO_VERSIONED flag must be set");
        // Version byte must be 3
        assert_eq!(raw[4], 3, "version byte must be 3");
    }

    #[test]
    fn test_decode_spotify_track_with_plugin_data() {
        let info = TrackInfo {
            identifier: "6rqhFgbbKwnb9MLmUQDhG6".to_string(),
            is_seekable: true,
            author: "Tame Impala".to_string(),
            length: 259400,
            is_stream: false,
            position: 0,
            title: "The Less I Know The Better".to_string(),
            uri: Some("https://open.spotify.com/track/6rqhFgbbKwnb9MLmUQDhG6".to_string()),
            artwork_url: None,
            isrc: Some("AUUM71400073".to_string()),
            source_name: "spotify".to_string(),
        };

        let track = Track::new(info);
        let decoded = Track::decode(&track.encoded).expect("decode should succeed");

        assert_eq!(decoded.info.identifier, "6rqhFgbbKwnb9MLmUQDhG6");
        assert_eq!(decoded.info.author, "Tame Impala");
        assert_eq!(decoded.info.length, 259400);
        assert_eq!(decoded.info.isrc.as_deref(), Some("AUUM71400073"));
        assert_eq!(decoded.info.source_name, "spotify");
        assert_eq!(decoded.info.artwork_url, None);
    }
}
