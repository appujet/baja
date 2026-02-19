use super::structs::*;

pub struct ProtoWriter {
    buffer: Vec<u8>,
}

impl ProtoWriter {
    pub fn new() -> Self {
        Self { buffer: Vec::new() }
    }

    fn push(&mut self, b: u8) {
        self.buffer.push(b);
    }

    fn push_bytes(&mut self, bytes: &[u8]) {
        self.buffer.extend_from_slice(bytes);
    }

    pub fn write_varint(&mut self, mut value: u64) {
        while value > 127 {
            self.push(((value & 0x7F) | 0x80) as u8);
            value >>= 7;
        }
        self.push(value as u8);
    }

    pub fn write_tag(&mut self, field_number: u32, wire_type: u8) {
        self.write_varint(((field_number << 3) | (wire_type as u32)) as u64);
    }

    pub fn write_string(&mut self, field_number: u32, s: &str) {
        if s.is_empty() {
            return;
        }
        self.write_tag(field_number, 2);
        self.write_varint(s.len() as u64);
        self.push_bytes(s.as_bytes());
    }

    pub fn write_bytes(&mut self, field_number: u32, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        self.write_tag(field_number, 2);
        self.write_varint(bytes.len() as u64);
        self.push_bytes(bytes);
    }

    pub fn write_int32(&mut self, field_number: u32, value: i32) {
        if value == 0 {
            return;
        }
        self.write_tag(field_number, 0);
        self.write_varint(value as u64);
    }

    pub fn write_uint32(&mut self, field_number: u32, value: u32) {
        if value == 0 {
            return;
        }
        self.write_tag(field_number, 0);
        self.write_varint(value as u64);
    }

    pub fn write_int64(&mut self, field_number: u32, value: i64) {
        if value == 0 {
            return;
        }
        self.write_tag(field_number, 0);
        self.write_varint(value as u64);
    }

    pub fn write_uint64(&mut self, field_number: u32, value: u64) {
        if value == 0 {
            return;
        }
        self.write_tag(field_number, 0);
        self.write_varint(value);
    }

    pub fn write_bool(&mut self, field_number: u32, value: bool) {
        if !value {
            return;
        }
        self.write_tag(field_number, 0);
        self.write_varint(1);
    }

    pub fn write_float(&mut self, field_number: u32, value: f32) {
        if value == 0.0 {
            return;
        }
        self.write_tag(field_number, 5);
        self.push_bytes(&value.to_le_bytes());
    }

    pub fn write_message<F>(&mut self, field_number: u32, f: F)
    where
        F: FnOnce(&mut ProtoWriter),
    {
        let mut writer = ProtoWriter::new();
        f(&mut writer);
        let bytes = writer.finish();
        if bytes.is_empty() {
            return;
        }
        self.write_tag(field_number, 2);
        self.write_varint(bytes.len() as u64);
        self.push_bytes(&bytes);
    }

    pub fn finish(self) -> Vec<u8> {
        self.buffer
    }
}

pub fn encode_format_id(msg: &FormatId, writer: &mut ProtoWriter) {
    writer.write_int32(1, msg.itag);
    if let Some(lm) = msg.last_modified {
        writer.write_int64(2, lm);
    }
    if let Some(xtags) = &msg.xtags {
        writer.write_string(3, xtags);
    }
}

pub fn encode_client_abr_state(msg: &ClientAbrState, writer: &mut ProtoWriter) {
    writer.write_int32(16, msg.last_manual_selected_resolution);
    writer.write_int32(21, msg.sticky_resolution);
    writer.write_bool(22, msg.client_viewport_is_flexible);
    writer.write_int64(23, msg.bandwidth_estimate);
    writer.write_int64(28, msg.player_time_ms);
    writer.write_int32(34, msg.visibility);
    writer.write_float(35, msg.playback_rate);
    writer.write_int64(39, msg.time_since_last_action_ms);
    writer.write_int32(40, msg.enabled_track_types_bitfield);
    writer.write_int64(44, msg.player_state);
    writer.write_bool(46, msg.drc_enabled);
    writer.write_string(69, &msg.audio_track_id);
}

pub fn encode_client_info(msg: &ClientInfo, writer: &mut ProtoWriter) {
    writer.write_int32(16, msg.client_name);
    writer.write_string(17, &msg.client_version);
}

pub fn encode_time_range(msg: &TimeRange, writer: &mut ProtoWriter) {
    writer.write_int64(1, msg.start_ticks);
    writer.write_int64(2, msg.duration_ticks);
    writer.write_int32(3, msg.timescale);
}

pub fn encode_buffered_range(msg: &BufferedRange, writer: &mut ProtoWriter) {
    if let Some(fid) = &msg.format_id {
        writer.write_message(1, |w| encode_format_id(fid, w));
    }
    writer.write_int64(2, msg.start_time_ms);
    writer.write_int64(3, msg.duration_ms);
    writer.write_int32(4, msg.start_segment_index);
    writer.write_int32(5, msg.end_segment_index);
    if let Some(tr) = &msg.time_range {
        writer.write_message(6, |w| encode_time_range(tr, w));
    }
}

pub fn encode_streamer_context(msg: &StreamerContext, writer: &mut ProtoWriter) {
    if let Some(info) = &msg.client_info {
        writer.write_message(1, |w| encode_client_info(info, w));
    }
    if let Some(token) = &msg.po_token {
        writer.write_bytes(2, token);
    }
    if let Some(cookie) = &msg.playback_cookie {
        writer.write_bytes(3, cookie);
    }
    for ctx in &msg.sabr_contexts {
        writer.write_message(5, |w| {
            w.write_int32(1, ctx.context_type);
            w.write_bytes(2, &ctx.value);
        });
    }
    for type_val in &msg.unsent_sabr_contexts {
        writer.write_int32(6, *type_val);
    }
}

pub fn encode_video_playback_abr_request(msg: &VideoPlaybackAbrRequest, writer: &mut ProtoWriter) {
    if let Some(state) = &msg.client_abr_state {
        writer.write_message(1, |w| encode_client_abr_state(state, w));
    }
    for fid in &msg.selected_format_ids {
        writer.write_message(2, |w| encode_format_id(fid, w));
    }
    for range in &msg.buffered_ranges {
        writer.write_message(3, |w| encode_buffered_range(range, w));
    }
    writer.write_int64(4, msg.player_time_ms);
    writer.write_bytes(5, &msg.video_playback_ustreamer_config);
    for fid in &msg.preferred_audio_format_ids {
        writer.write_message(16, |w| encode_format_id(fid, w));
    }
    for fid in &msg.preferred_video_format_ids {
        writer.write_message(17, |w| encode_format_id(fid, w));
    }
    if let Some(ctx) = &msg.streamer_context {
        writer.write_message(19, |w| encode_streamer_context(ctx, w));
    }
}
