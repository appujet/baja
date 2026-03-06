use base64::{Engine, prelude::BASE64_STANDARD};
use byteorder::{BigEndian, ReadBytesExt};

use crate::protocol::{
    CodecError, PlaylistInfo, Track, TrackInfo,
    codec::io::{BinaryBuffer, V1, V2, V3},
};

pub fn decode_track(encoded: &str) -> Result<Track, CodecError> {
    if encoded.is_empty() {
        return Err(CodecError::EmptyInput);
    }

    let raw_payload = BASE64_STANDARD.decode(encoded)?;
    if raw_payload.len() < 4 {
        return Err(CodecError::TruncatedBuffer("header".into()));
    }

    let mut stream = BinaryBuffer::from_data(raw_payload);
    let envelope = stream.read_u32::<BigEndian>()?;
    let flags = (envelope >> 30) & 0x03;

    let ver = if flags & 1 != 0 {
        stream.read_u8()?
    } else {
        V1
    };

    if !(V1..=V3).contains(&ver) {
        return Err(CodecError::UnknownVersion(ver));
    }

    let title = stream.read_string()?;
    let author = stream.read_string()?;
    let length = stream.read_u64::<BigEndian>()?;
    let identifier = stream.read_string()?;
    let is_stream = stream.read_u8()? != 0;

    let (uri, artwork_url, isrc) = match ver {
        V2 => (stream.read_nullable_string()?, None, None),
        V3 => (
            stream.read_nullable_string()?,
            stream.read_nullable_string()?,
            stream.read_nullable_string()?,
        ),
        _ => (None, None, None),
    };

    let source_name = stream.read_string()?;
    let position = stream.read_u64::<BigEndian>().unwrap_or(0);

    // Optional userData follows the standard fields
    let user_data = stream.read_json().unwrap_or_else(|_| serde_json::json!({}));

    Ok(Track {
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
        user_data,
    })
}

pub fn decode_playlist_info(encoded: &str) -> Result<PlaylistInfo, CodecError> {
    let raw = BASE64_STANDARD.decode(encoded)?;
    let mut stream = BinaryBuffer::from_data(raw);

    Ok(PlaylistInfo {
        name: stream.read_string()?,
        selected_track: stream.read_i32::<BigEndian>()?,
    })
}
