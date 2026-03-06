use std::io::Write;

use base64::{Engine, prelude::BASE64_STANDARD};
use byteorder::{BigEndian, WriteBytesExt};

use crate::protocol::{
    CodecError, PlaylistInfo, TrackInfo,
    codec::io::{BinaryBuffer, V3},
};

/// Serializes TrackInfo and optional userData into a base64 encoded token.
pub fn encode_track(
    metadata: &TrackInfo,
    user_data: &serde_json::Value,
) -> Result<String, CodecError> {
    let mut blob = BinaryBuffer::with_capacity(128);

    // Force version 3 for modern compatibility
    blob.write_u8(V3)?;
    blob.write_string(&metadata.title)?;
    blob.write_string(&metadata.author)?;
    blob.write_u64::<BigEndian>(metadata.length)?;
    blob.write_string(&metadata.identifier)?;
    blob.write_u8(u8::from(metadata.is_stream))?;

    blob.write_nullable_string(metadata.uri.as_deref())?;
    blob.write_nullable_string(metadata.artwork_url.as_deref())?;
    blob.write_nullable_string(metadata.isrc.as_deref())?;

    blob.write_string(&metadata.source_name)?;
    blob.write_u64::<BigEndian>(metadata.position)?;

    // Append userData if it's not null and not empty
    if let Some(obj) = user_data.as_object() {
        if !obj.is_empty() {
            blob.write_json(user_data)?;
        }
    } else if !user_data.is_null() {
        blob.write_json(user_data)?;
    }

    let inner = blob.into_inner();
    let header = (inner.len() as u32) | (1u32 << 30);

    let mut out = Vec::with_capacity(4 + inner.len());
    out.write_u32::<BigEndian>(header)?;
    out.write_all(&inner)?;

    Ok(BASE64_STANDARD.encode(&out))
}

/// Serializes PlaylistInfo into a base64 encoded token.
pub fn encode_playlist_info(info: &PlaylistInfo) -> Result<String, CodecError> {
    let mut blob = BinaryBuffer::with_capacity(64);
    blob.write_string(&info.name)?;
    blob.write_i32::<BigEndian>(info.selected_track)?;
    Ok(BASE64_STANDARD.encode(blob.into_inner()))
}
