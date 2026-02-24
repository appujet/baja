//! Hand-rolled protobuf encoder/decoder and UMP varint parser for SABR.
//!
//! No external protobuf crate needed — this matches exactly what the
//! protor.js and youtube-source's SabrAudioStream.java do.

// ─── Protobuf Writer ───────────────────────────────────────────────────────

pub struct ProtoWriter {
    buf: Vec<u8>,
}

impl ProtoWriter {
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    // Write a protobuf varint (7-bit continuation encoding)
    pub fn write_varint(&mut self, mut v: u64) {
        while v > 0x7F {
            self.buf.push((v as u8 & 0x7F) | 0x80);
            v >>= 7;
        }
        self.buf.push(v as u8);
    }

    fn write_tag(&mut self, field: u32, wire_type: u8) {
        self.write_varint(((field as u64) << 3) | (wire_type as u64));
    }

    /// Field type 0 (varint), skip if value == 0
    pub fn write_i32(&mut self, field: u32, value: i32) {
        if value == 0 { return; }
        self.write_tag(field, 0);
        self.write_varint(value as u64);
    }

    /// Field type 0 (varint), skip if value == 0
    pub fn write_i64(&mut self, field: u32, value: i64) {
        if value == 0 { return; }
        self.write_tag(field, 0);
        self.write_varint(value as u64);
    }

    /// Field type 0 (varint), skip if value == 0
    pub fn write_u64(&mut self, field: u32, value: u64) {
        if value == 0 { return; }
        self.write_tag(field, 0);
        self.write_varint(value);
    }

    /// Field type 0, always write (even if 0) — for BigInt-style values
    pub fn write_u64_always(&mut self, field: u32, value: u64) {
        self.write_tag(field, 0);
        self.write_varint(value);
    }

    /// Field type 0, write bool if true
    pub fn write_bool(&mut self, field: u32, value: bool) {
        if !value { return; }
        self.write_tag(field, 0);
        self.buf.push(1);
    }

    /// Field type 5 (32-bit LE float)
    pub fn write_float(&mut self, field: u32, value: f32) {
        if value == 0.0 { return; }
        self.write_tag(field, 5);
        self.buf.extend_from_slice(&value.to_le_bytes());
    }

    /// Field type 2 (length-delimited string)
    pub fn write_string(&mut self, field: u32, value: &str) {
        if value.is_empty() { return; }
        self.write_tag(field, 2);
        self.write_varint(value.len() as u64);
        self.buf.extend_from_slice(value.as_bytes());
    }

    /// Field type 2 (length-delimited bytes)
    pub fn write_bytes(&mut self, field: u32, value: &[u8]) {
        if value.is_empty() { return; }
        self.write_tag(field, 2);
        self.write_varint(value.len() as u64);
        self.buf.extend_from_slice(value);
    }

    /// Embed a nested message (field type 2)
    pub fn write_message(&mut self, field: u32, nested: ProtoWriter) {
        let bytes = nested.finish();
        if bytes.is_empty() { return; }
        self.write_tag(field, 2);
        self.write_varint(bytes.len() as u64);
        self.buf.extend_from_slice(&bytes);
    }

    pub fn finish(self) -> Vec<u8> {
        self.buf
    }
}

// ─── Protobuf Reader ───────────────────────────────────────────────────────

pub struct ProtoReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> ProtoReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub fn has_remaining(&self) -> bool {
        self.pos < self.data.len()
    }

    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    pub fn read_varint(&mut self) -> u64 {
        let mut result: u64 = 0;
        let mut shift = 0u64;
        while self.pos < self.data.len() {
            let b = self.data[self.pos] as u64;
            self.pos += 1;
            result |= (b & 0x7F) << shift;
            shift += 7;
            if b & 0x80 == 0 { break; }
        }
        result
    }

    pub fn read_tag(&mut self) -> u32 {
        self.read_varint() as u32
    }

    pub fn read_string(&mut self) -> String {
        let len = self.read_varint() as usize;
        if self.pos + len > self.data.len() {
            return String::new();
        }
        let s = std::str::from_utf8(&self.data[self.pos..self.pos + len])
            .unwrap_or("")
            .to_string();
        self.pos += len;
        s
    }

    pub fn read_bytes(&mut self) -> &'a [u8] {
        let len = self.read_varint() as usize;
        if self.pos + len > self.data.len() {
            return &[];
        }
        let bytes = &self.data[self.pos..self.pos + len];
        self.pos += len;
        bytes
    }

    pub fn read_length_delimited(&mut self) -> &'a [u8] {
        self.read_bytes()
    }

    pub fn skip_field(&mut self, wire_type: u32) {
        match wire_type {
            0 => { self.read_varint(); }
            1 => { self.pos = (self.pos + 8).min(self.data.len()); }
            2 => { let len = self.read_varint() as usize; self.pos = (self.pos + len).min(self.data.len()); }
            5 => { self.pos = (self.pos + 4).min(self.data.len()); }
            _ => { self.pos = self.data.len(); } // Unknown — consume all
        }
    }
}

// ─── UMP Varint ────────────────────────────────────────────────────────────

// UMP uses a *different* multi-byte encoding than protobuf varint:
//  - 0x00..0x7F  (1 byte):  value = byte[0]
//  - 0x80..0xBF  (2 bytes): value = (byte[0] & 0x3F) + 64 * byte[1]
//  - 0xC0..0xDF  (3 bytes): value = (byte[0] & 0x1F) + 32 * (byte[1] + 256 * byte[2])
//  - 0xE0..0xEF  (4 bytes): value = (byte[0] & 0x0F) + 16 * (byte[1] + 256 * (byte[2] + 256 * byte[3]))
//  - 0xF0..      (5 bytes): skip byte[0], read LE u32 from bytes[1..5]

/// Parse one UMP varint from a byte slice, returning `(value, bytes_consumed)`.
/// Returns `None` if not enough bytes are available.
pub fn read_ump_varint(data: &[u8], offset: usize) -> Option<(u64, usize)> {
    if offset >= data.len() {
        return None;
    }
    let first = data[offset] as u64;

    if first < 0x80 {
        return Some((first, 1));
    }
    if first < 0xC0 {
        if offset + 2 > data.len() { return None; }
        let b2 = data[offset + 1] as u64;
        return Some(((first & 0x3F) + 64 * b2, 2));
    }
    if first < 0xE0 {
        if offset + 3 > data.len() { return None; }
        let b2 = data[offset + 1] as u64;
        let b3 = data[offset + 2] as u64;
        return Some(((first & 0x1F) + 32 * (b2 + 256 * b3), 3));
    }
    if first < 0xF0 {
        if offset + 4 > data.len() { return None; }
        let b2 = data[offset + 1] as u64;
        let b3 = data[offset + 2] as u64;
        let b4 = data[offset + 3] as u64;
        return Some(((first & 0x0F) + 16 * (b2 + 256 * (b3 + 256 * b4)), 4));
    }
    // 5-byte form
    if offset + 5 > data.len() { return None; }
    let value = u32::from_le_bytes([
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
        data[offset + 4],
    ]) as u64;
    Some((value, 5))
}

// ─── Message Encoders ──────────────────────────────────────────────────────

/// Encode a FormatId proto sub-message (fields 1=itag, 2=lastModified, 3=xtags)
pub fn encode_format_id(itag: i32, last_modified: &str, xtags: Option<&str>) -> ProtoWriter {
    let mut w = ProtoWriter::new();
    w.write_i32(1, itag);
    // lastModified is a string-encoded u64 from the API
    if let Ok(lm) = last_modified.parse::<u64>() {
        w.write_u64(2, lm);
    }
    if let Some(xt) = xtags {
        w.write_string(3, xt);
    }
    w
}

/// Encode the ClientInfo sub-message (field 16=clientName int, field 17=clientVersion string)
pub fn encode_client_info(client_name_id: i32, client_version: &str) -> ProtoWriter {
    let mut w = ProtoWriter::new();
    w.write_i32(16, client_name_id);
    w.write_string(17, client_version);
    w
}

/// Encode the StreamerContext sub-message (field 19 of VideoPlaybackAbrRequest)
pub fn encode_streamer_context(
    client_name_id: i32,
    client_version: &str,
    po_token: Option<&[u8]>,
    playback_cookie: Option<&[u8]>,
    sabr_contexts: &[(i32, Vec<u8>)],       // (type, value) pairs to send
    unsent_context_types: &[i32],            // types to report as unsent
) -> ProtoWriter {
    let mut w = ProtoWriter::new();
    w.write_message(1, encode_client_info(client_name_id, client_version));
    if let Some(pt) = po_token {
        w.write_bytes(2, pt);
    }
    if let Some(cookie) = playback_cookie {
        w.write_bytes(3, cookie);
    }
    for (ctx_type, ctx_value) in sabr_contexts {
        let mut ctx_w = ProtoWriter::new();
        ctx_w.write_i32(1, *ctx_type);
        ctx_w.write_bytes(2, ctx_value);
        w.write_message(5, ctx_w);
    }
    for t in unsent_context_types {
        w.write_i32(6, *t);
    }
    w
}

/// Encode the full VideoPlaybackAbrRequest body.
#[allow(clippy::too_many_arguments)]
pub fn encode_video_playback_abr_request(
    // ClientAbrState fields
    bandwidth_estimate: u64,
    player_time_ms: u64,
    enabled_track_types_bitfield: i32,   // 1 = audio only
    audio_track_id: &str,
    // Format IDs
    selected_format_ids: &[(i32, &str, Option<&str>)],  // (itag, lastMod, xtags)
    buffered_ranges: &[EncodedBufferedRange],
    preferred_audio_format_ids: &[(i32, &str, Option<&str>)],
    // Core config
    ustreamer_config: &[u8],
    // StreamerContext
    client_name_id: i32,
    client_version: &str,
    po_token: Option<&[u8]>,
    playback_cookie: Option<&[u8]>,
    sabr_contexts: &[(i32, Vec<u8>)],
    unsent_context_types: &[i32],
    player_state: u64,
) -> Vec<u8> {
    let mut root = ProtoWriter::new();

    // Field 1: ClientAbrState
    {
        let mut s = ProtoWriter::new();
        s.write_i32(16, 1080);                               // lastManualSelectedResolution
        s.write_i32(21, 1080);                               // stickyResolution
        s.write_bool(22, false);                             // clientViewportIsFlexible
        s.write_u64(23, bandwidth_estimate.max(500_000));    // bandwidthEstimate
        s.write_u64(28, player_time_ms);                     // playerTimeMs
        s.write_i32(34, 1);                                  // visibility
        s.write_float(35, 1.0);                              // playbackRate
        s.write_i64(39, 0);                                  // timeSinceLastActionMs
        s.write_i32(40, enabled_track_types_bitfield);       // enabledTrackTypesBitfield
        s.write_u64(44, player_state);                       // playerState
        s.write_bool(46, false);                             // drcEnabled
        s.write_string(69, audio_track_id);                  // audioTrackId
        root.write_message(1, s);
    }

    // Field 2: selectedFormatIds (one per format)
    for (itag, lm, xtags) in selected_format_ids {
        root.write_message(2, encode_format_id(*itag, lm, *xtags));
    }

    // Field 3: bufferedRanges
    for br in buffered_ranges {
        root.write_message(3, br.encode());
    }

    // Field 4: playerTimeMs (root level)
    root.write_u64(4, player_time_ms);

    // Field 5: videoPlaybackUstreamerConfig bytes
    root.write_bytes(5, ustreamer_config);

    // Field 16: preferredAudioFormatIds
    for (itag, lm, xtags) in preferred_audio_format_ids {
        root.write_message(16, encode_format_id(*itag, lm, *xtags));
    }

    // Field 19: StreamerContext
    root.write_message(
        19,
        encode_streamer_context(
            client_name_id,
            client_version,
            po_token,
            playback_cookie,
            sabr_contexts,
            unsent_context_types,
        ),
    );

    root.finish()
}

// ─── BufferedRange encoder ─────────────────────────────────────────────────

#[derive(Clone)]
pub struct EncodedBufferedRange {
    pub itag: i32,
    pub last_modified: String,
    pub xtags: Option<String>,
    pub start_time_ms: u64,
    pub duration_ms: u64,
    pub start_segment_index: u32,
    pub end_segment_index: u32,
    pub timescale: u32,
}

impl EncodedBufferedRange {
    pub fn encode(&self) -> ProtoWriter {
        let mut w = ProtoWriter::new();
        // field 1: FormatId
        w.write_message(
            1,
            encode_format_id(self.itag, &self.last_modified, self.xtags.as_deref()),
        );
        w.write_u64(2, self.start_time_ms);           // startTimeMs
        w.write_u64(3, self.duration_ms);             // durationMs
        w.write_i32(4, self.start_segment_index as i32); // startSegmentIndex
        w.write_i32(5, self.end_segment_index as i32);   // endSegmentIndex
        // field 6: TimeRange
        if self.timescale > 0 {
            let mut tr = ProtoWriter::new();
            let dur_ticks = (self.duration_ms as u128 * self.timescale as u128) / 1000;
            let start_ticks = (self.start_time_ms as u128 * self.timescale as u128) / 1000;
            tr.write_u64(1, start_ticks as u64);
            tr.write_u64(2, dur_ticks as u64);
            tr.write_i32(3, self.timescale as i32);
            w.write_message(6, tr);
        }
        w
    }
}

// ─── UMP Part Decoders ─────────────────────────────────────────────────────

pub const UMP_FORMAT_INITIALIZATION_METADATA: u64 = 42;
pub const UMP_NEXT_REQUEST_POLICY: u64 = 35;
pub const UMP_SABR_ERROR: u64 = 44;
pub const UMP_SABR_REDIRECT: u64 = 43;
pub const UMP_RELOAD_PLAYER_RESPONSE: u64 = 46;
pub const UMP_SABR_CONTEXT_UPDATE: u64 = 57;
pub const UMP_STREAM_PROTECTION_STATUS: u64 = 58;
pub const UMP_SABR_CONTEXT_SENDING_POLICY: u64 = 59;
pub const UMP_MEDIA_HEADER: u64 = 20;
pub const UMP_MEDIA: u64 = 21;
pub const UMP_MEDIA_END: u64 = 22;

#[derive(Debug, Clone)]
pub struct MediaHeaderMsg {
    pub header_id: u8,
    pub itag: i32,
    pub xtags: Option<String>,
    pub is_init_seg: bool,
    pub sequence_number: u32,
    pub start_ms: u64,
    pub duration_ms: u64,
    pub timescale: u32,
    pub duration_ticks: u64,
    pub start_ticks: u64,
    pub last_modified: String,
}

pub fn decode_media_header(data: &[u8]) -> Option<MediaHeaderMsg> {
    let mut r = ProtoReader::new(data);
    let mut h = MediaHeaderMsg {
        header_id: 0,
        itag: 0,
        xtags: None,
        is_init_seg: false,
        sequence_number: 0,
        start_ms: 0,
        duration_ms: 0,
        timescale: 0,
        duration_ticks: 0,
        start_ticks: 0,
        last_modified: "0".to_string(),
    };

    while r.has_remaining() {
        let tag = r.read_tag();
        let field = tag >> 3;
        let wire = tag & 7;
        match field {
            1 if wire == 0  => h.header_id = r.read_varint() as u8,
            3 if wire == 0  => h.itag = r.read_varint() as i32,
            4 if wire == 0  => h.last_modified = r.read_varint().to_string(),
            5 if wire == 2  => h.xtags = Some(r.read_string()),
            8 if wire == 0  => h.is_init_seg = r.read_varint() != 0,
            9 if wire == 0  => h.sequence_number = r.read_varint() as u32,
            11 if wire == 0 => h.start_ms = r.read_varint(),
            12 if wire == 0 => h.duration_ms = r.read_varint(),
            13 if wire == 2 => {
                // FormatId sub-message
                let nested = r.read_length_delimited();
                let mut nr = ProtoReader::new(nested);
                while nr.has_remaining() {
                    let t = nr.read_tag();
                    match t >> 3 {
                        1 => h.itag = nr.read_varint() as i32,
                        2 => h.last_modified = nr.read_varint().to_string(),
                        3 => h.xtags = Some(nr.read_string()),
                        _ => nr.skip_field(t & 7),
                    }
                }
            }
            15 if wire == 2 => {
                // TimeRange sub-message
                let nested = r.read_length_delimited();
                let mut nr = ProtoReader::new(nested);
                while nr.has_remaining() {
                    let t = nr.read_tag();
                    match t >> 3 {
                        1 => h.start_ticks = nr.read_varint(),
                        2 => h.duration_ticks = nr.read_varint(),
                        3 => h.timescale = nr.read_varint() as u32,
                        _ => nr.skip_field(t & 7),
                    }
                }
            }
            _ => r.skip_field(wire),
        }
    }

    if h.itag == 0 { None } else { Some(h) }
}

#[derive(Debug, Clone)]
pub struct FormatInitMsg {
    pub itag: i32,
    pub xtags: Option<String>,
    pub last_modified: String,
    pub end_segment_number: Option<u32>,
    pub mime_type: String,
}

pub fn decode_format_init_metadata(data: &[u8]) -> Option<FormatInitMsg> {
    let mut r = ProtoReader::new(data);
    let mut m = FormatInitMsg {
        itag: 0,
        xtags: None,
        last_modified: "0".to_string(),
        end_segment_number: None,
        mime_type: String::new(),
    };

    while r.has_remaining() {
        let tag = r.read_tag();
        let field = tag >> 3;
        let wire = tag & 7;
        match field {
            2 if wire == 2 => {
                let nested = r.read_length_delimited();
                let mut nr = ProtoReader::new(nested);
                while nr.has_remaining() {
                    let t = nr.read_tag();
                    match t >> 3 {
                        1 => m.itag = nr.read_varint() as i32,
                        2 => m.last_modified = nr.read_varint().to_string(),
                        3 => m.xtags = Some(nr.read_string()),
                        _ => nr.skip_field(t & 7),
                    }
                }
            }
            4 if wire == 0 => m.end_segment_number = Some(nr_varint(&mut r) as u32),
            5 if wire == 2 => m.mime_type = r.read_string(),
            _ => r.skip_field(wire),
        }
    }

    if m.itag == 0 { None } else { Some(m) }
}

// We need a small helper since we can't inline the read inside a match arm easily
fn nr_varint(r: &mut ProtoReader) -> u64 {
    r.read_varint()
}

#[derive(Debug, Clone)]
pub struct NextRequestPolicyMsg {
    pub backoff_ms: u64,
    pub playback_cookie: Option<Vec<u8>>,
}

pub fn decode_next_request_policy(data: &[u8]) -> NextRequestPolicyMsg {
    let mut r = ProtoReader::new(data);
    let mut m = NextRequestPolicyMsg {
        backoff_ms: 0,
        playback_cookie: None,
    };

    while r.has_remaining() {
        let tag = r.read_tag();
        let field = tag >> 3;
        let wire = tag & 7;
        match field {
            4 if wire == 0 => m.backoff_ms = r.read_varint(),
            7 if wire == 2 => m.playback_cookie = Some(r.read_bytes().to_vec()),
            _ => r.skip_field(wire),
        }
    }
    m
}

#[derive(Debug)]
pub struct SabrErrorMsg {
    pub error_type: String,
    pub code: i32,
}

pub fn decode_sabr_error(data: &[u8]) -> SabrErrorMsg {
    let mut r = ProtoReader::new(data);
    let mut m = SabrErrorMsg { error_type: String::new(), code: 0 };
    while r.has_remaining() {
        let tag = r.read_tag();
        match tag >> 3 {
            1 => m.error_type = r.read_string(),
            2 => m.code = r.read_varint() as i32,
            _ => r.skip_field(tag & 7),
        }
    }
    m
}

pub fn decode_sabr_redirect(data: &[u8]) -> Option<String> {
    let mut r = ProtoReader::new(data);
    while r.has_remaining() {
        let tag = r.read_tag();
        match tag >> 3 {
            1 => return Some(r.read_string()),
            _ => r.skip_field(tag & 7),
        }
    }
    None
}

#[derive(Debug, Clone)]
pub struct SabrContextUpdateMsg {
    pub context_type: i32,
    pub value: Vec<u8>,
    pub send_by_default: bool,
}

pub fn decode_sabr_context_update(data: &[u8]) -> Option<SabrContextUpdateMsg> {
    let mut r = ProtoReader::new(data);
    let mut m = SabrContextUpdateMsg { context_type: -1, value: Vec::new(), send_by_default: false };

    while r.has_remaining() {
        let tag = r.read_tag();
        let field = tag >> 3;
        let wire = tag & 7;
        match field {
            1 if wire == 0 => m.context_type = r.read_varint() as i32,
            3 if wire == 2 => m.value = r.read_bytes().to_vec(),
            4 if wire == 0 => m.send_by_default = r.read_varint() != 0,
            _ => r.skip_field(wire),
        }
    }

    if m.context_type < 0 || m.value.is_empty() { None } else { Some(m) }
}

#[derive(Debug, Clone)]
pub struct SabrContextSendingPolicyMsg {
    pub start_policy: Vec<i32>,
    pub stop_policy: Vec<i32>,
    pub discard_policy: Vec<i32>,
}

pub fn decode_sabr_context_sending_policy(data: &[u8]) -> SabrContextSendingPolicyMsg {
    let mut r = ProtoReader::new(data);
    let mut m = SabrContextSendingPolicyMsg {
        start_policy: Vec::new(),
        stop_policy: Vec::new(),
        discard_policy: Vec::new(),
    };

    while r.has_remaining() {
        let tag = r.read_tag();
        let field = tag >> 3;
        let wire = tag & 7;
        match field {
            1 if wire == 0 => m.start_policy.push(r.read_varint() as i32),
            2 if wire == 0 => m.stop_policy.push(r.read_varint() as i32),
            3 if wire == 0 => m.discard_policy.push(r.read_varint() as i32),
            _ => r.skip_field(wire),
        }
    }
    m
}

pub fn decode_stream_protection_status(data: &[u8]) -> i32 {
    let mut r = ProtoReader::new(data);
    while r.has_remaining() {
        let tag = r.read_tag();
        match tag >> 3 {
            1 => return r.read_varint() as i32,
            _ => r.skip_field(tag & 7),
        }
    }
    0
}

// ─── UMP parser ────────────────────────────────────────────────────────────

/// One parsed UMP part.
pub struct UmpPart<'a> {
    pub part_type: u64,
    pub data: &'a [u8],
}

/// Parse all complete UMP parts from `data`.
/// Returns an iterator-style vec of `(type, payload_slice)`.
pub fn parse_ump_parts(data: &[u8]) -> Vec<(u64, &[u8])> {
    let mut parts = Vec::new();
    let mut offset = 0;

    while offset < data.len() {
        let Some((part_type, n1)) = read_ump_varint(data, offset) else { break };
        offset += n1;

        let Some((part_size, n2)) = read_ump_varint(data, offset) else { break };
        offset += n2;

        let size = part_size as usize;
        if offset + size > data.len() {
            break;
        }
        parts.push((part_type, &data[offset..offset + size]));
        offset += size;
    }

    parts
}
/// Stateful UMP parser for streaming byte chunks.
pub struct UmpStreamParser {
    buffer: Vec<u8>,
}

impl UmpStreamParser {
    pub fn new() -> Self {
        Self {
            buffer: Vec::new(),
        }
    }

    /// Push a new byte chunk and return all complete UMP parts.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<(u64, Vec<u8>)> {
        self.buffer.extend_from_slice(chunk);
        let mut parts = Vec::new();
        let mut offset = 0;

        while offset < self.buffer.len() {
            let Some((part_type, n1)) = read_ump_varint(&self.buffer, offset) else {
                break;
            };
            let size_offset = offset + n1;

            let Some((part_size, n2)) = read_ump_varint(&self.buffer, size_offset) else {
                break;
            };
            let data_offset = size_offset + n2;
            let size = part_size as usize;

            if data_offset + size > self.buffer.len() {
                // Incomplete part
                break;
            }

            let payload = self.buffer[data_offset..data_offset + size].to_vec();
            parts.push((part_type, payload));
            offset = data_offset + size;
        }

        if offset > 0 {
            self.buffer.drain(..offset);
        }

        parts
    }
}
