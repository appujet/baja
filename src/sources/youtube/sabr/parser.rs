// No top-level imports needed if only submodules use them,
// but wait, I see ProtoReader used in submodules.
// Actually, I'll just remove the unused ones.

pub struct ProtoReader<'a> {
    buffer: &'a [u8],
    pub pos: usize,
}

impl<'a> ProtoReader<'a> {
    pub fn new(buffer: &'a [u8]) -> Self {
        Self { buffer, pos: 0 }
    }

    pub fn read_varint(&mut self) -> Option<u64> {
        let mut result: u64 = 0;
        let mut shift = 0;
        loop {
            if self.pos >= self.buffer.len() {
                return None;
            }
            let b = self.buffer[self.pos];
            self.pos += 1;
            result |= ((b & 0x7F) as u64) << shift;
            shift += 7;
            if (b & 0x80) == 0 {
                return Some(result);
            }
            if shift >= 64 {
                // Return what we have to avoid infinite loop
                return Some(result);
            }
        }
    }

    pub fn read_string(&mut self) -> String {
        let len = self.read_varint().unwrap_or(0) as usize;
        if self.pos + len > self.buffer.len() {
            return String::new();
        }
        let s = String::from_utf8_lossy(&self.buffer[self.pos..self.pos + len]).to_string();
        self.pos += len;
        s
    }

    pub fn read_bytes(&mut self) -> Vec<u8> {
        let len = self.read_varint().unwrap_or(0) as usize;
        if self.pos + len > self.buffer.len() {
            return Vec::new();
        }
        let b = self.buffer[self.pos..self.pos + len].to_vec();
        self.pos += len;
        b
    }

    pub fn skip(&mut self, wire_type: u8) {
        if self.pos >= self.buffer.len() {
            return;
        }
        match wire_type {
            0 => {
                self.read_varint();
            }
            1 => {
                self.pos = std::cmp::min(self.pos + 8, self.buffer.len());
            }
            2 => {
                let len = self.read_varint().unwrap_or(0) as usize;
                self.pos = std::cmp::min(self.pos + len, self.buffer.len());
            }
            5 => {
                self.pos = std::cmp::min(self.pos + 4, self.buffer.len());
            }
            _ => {}
        }
    }
}

pub struct UmpReader<'a> {
    reader: ProtoReader<'a>,
}

pub struct UmpPart {
    pub part_type: u64,
    pub data: Vec<u8>,
}

impl<'a> UmpReader<'a> {
    pub fn new(buffer: &'a [u8]) -> Self {
        Self {
            reader: ProtoReader::new(buffer),
        }
    }

    pub fn next_part(&mut self) -> Option<UmpPart> {
        if self.reader.pos >= self.reader.buffer.len() {
            return None;
        }
        let part_type = self.reader.read_varint()?;
        let length = self.reader.read_varint()? as usize;

        if self.reader.pos + length > self.reader.buffer.len() {
            return None;
        }

        let data = self.reader.buffer[self.reader.pos..self.reader.pos + length].to_vec();
        self.reader.pos += length;

        Some(UmpPart { part_type, data })
    }
}

pub mod decoders {
    use super::ProtoReader;
    use crate::sources::youtube::sabr::structs::*;

    pub fn decode_format_id(reader: &mut ProtoReader, len: usize) -> FormatId {
        let end = reader.pos + len;
        let mut msg = FormatId::default();
        while reader.pos < end {
            let tag = reader.read_varint().unwrap_or(0);
            if tag == 0 {
                break;
            }
            let field = tag >> 3;
            let wire_type = (tag & 7) as u8;
            match field {
                1 => msg.itag = reader.read_varint().unwrap_or(0) as i32,
                2 => msg.last_modified = Some(reader.read_varint().unwrap_or(0) as i64),
                3 => msg.xtags = Some(reader.read_string()),
                _ => reader.skip(wire_type),
            }
        }
        msg
    }

    pub fn decode_media_header(reader: &mut ProtoReader, len: usize) -> MediaHeader {
        let end = reader.pos + len;
        let mut msg = MediaHeader::default();
        while reader.pos < end {
            let tag = reader.read_varint().unwrap_or(0);
            if tag == 0 {
                break;
            }
            let field = tag >> 3;
            let wire_type = (tag & 7) as u8;
            match field {
                1 => msg.header_id = reader.read_varint().unwrap_or(0) as i32,
                3 => msg.itag = reader.read_varint().unwrap_or(0) as i32,
                4 => msg.lmt = Some(reader.read_varint().unwrap_or(0).to_string()),
                5 => msg.xtags = Some(reader.read_string()),
                8 => msg.is_init_seg = reader.read_varint().unwrap_or(0) != 0,
                9 => msg.sequence_number = reader.read_varint().unwrap_or(0) as i32,
                11 => msg.start_ms = reader.read_varint().unwrap_or(0).to_string(),
                12 => msg.duration_ms = reader.read_varint().unwrap_or(0).to_string(),
                13 => {
                    let sub_len = reader.read_varint().unwrap_or(0) as usize;
                    msg.format_id = Some(decode_format_id(reader, sub_len));
                }
                14 => msg.content_length = Some(reader.read_varint().unwrap_or(0).to_string()),
                _ => reader.skip(wire_type),
            }
        }
        msg
    }

    pub fn decode_next_request_policy(reader: &mut ProtoReader, len: usize) -> NextRequestPolicy {
        let end = reader.pos + len;
        let mut msg = NextRequestPolicy::default();
        while reader.pos < end {
            let tag = reader.read_varint().unwrap_or(0);
            if tag == 0 {
                break;
            }
            let field = tag >> 3;
            let wire_type = (tag & 7) as u8;
            match field {
                1 => msg.target_audio_readahead_ms = reader.read_varint().unwrap_or(0) as i32,
                2 => msg.target_video_readahead_ms = reader.read_varint().unwrap_or(0) as i32,
                3 => msg.max_time_since_last_request_ms = reader.read_varint().unwrap_or(0) as i32,
                4 => msg.backoff_time_ms = reader.read_varint().unwrap_or(0) as i32,
                7 => msg.playback_cookie = Some(reader.read_bytes()),
                _ => reader.skip(wire_type),
            }
        }
        msg
    }

    pub fn decode_format_initialization_metadata(
        reader: &mut ProtoReader,
        len: usize,
    ) -> FormatInitializationMetadata {
        let end = reader.pos + len;
        let mut msg = FormatInitializationMetadata::default();
        while reader.pos < end {
            let tag = reader.read_varint().unwrap_or(0);
            if tag == 0 {
                break;
            }
            let field = tag >> 3;
            let wire_type = (tag & 7) as u8;
            match field {
                2 => {
                    let sub_len = reader.read_varint().unwrap_or(0) as usize;
                    msg.format_id = Some(decode_format_id(reader, sub_len));
                    if let Some(fid) = &msg.format_id {
                        msg.itag = Some(fid.itag);
                    }
                }
                4 => msg.end_segment_number = reader.read_varint().unwrap_or(0).to_string(),
                5 => msg.mime_type = reader.read_string(),
                9 => msg.duration_units = reader.read_varint().unwrap_or(0).to_string(),
                10 => msg.duration_timescale = reader.read_varint().unwrap_or(0).to_string(),
                _ => reader.skip(wire_type),
            }
        }
        msg
    }

    pub fn decode_sabr_redirect(reader: &mut ProtoReader, len: usize) -> SabrRedirect {
        let end = reader.pos + len;
        let mut msg = SabrRedirect::default();
        while reader.pos < end {
            let tag = reader.read_varint().unwrap_or(0);
            if tag == 0 {
                break;
            }
            let field = tag >> 3;
            let wire_type = (tag & 7) as u8;
            match field {
                1 => msg.url = reader.read_string(),
                _ => reader.skip(wire_type),
            }
        }
        msg
    }

    pub fn decode_sabr_error(reader: &mut ProtoReader, len: usize) -> SabrError {
        let end = reader.pos + len;
        let mut msg = SabrError::default();
        while reader.pos < end {
            let tag = reader.read_varint().unwrap_or(0);
            if tag == 0 {
                break;
            }
            let field = tag >> 3;
            let wire_type = (tag & 7) as u8;
            match field {
                1 => msg.error_type = reader.read_string(),
                2 => msg.code = reader.read_varint().unwrap_or(0) as i32,
                _ => reader.skip(wire_type),
            }
        }
        msg
    }
}
