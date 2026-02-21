/// Minimal MPEG-TS demuxer that extracts AAC/ADTS elementary stream data.
///
/// MPEG-TS files consist of 188-byte packets. Each contains a 4-byte header
/// followed by optional adaptation field and payload. We parse PAT → PMT to
/// identify the audio PID, then extract PES payloads from that PID, stripping
/// PES headers to yield raw ADTS frames consumable by symphonia.
///
/// This mirrors Lavaplayer's `MpegTsElementaryInputStream` + `PesPacketInputStream`.

const TS_PACKET_SIZE: usize = 188;
const TS_SYNC_BYTE: u8 = 0x47;
const PAT_PID: u16 = 0x0000;

/// AAC stream types in MPEG-TS PMT
const STREAM_TYPE_AAC: u8 = 0x0F; // ISO/IEC 13818-7 (ADTS AAC)
const STREAM_TYPE_AAC_LATM: u8 = 0x11; // ISO/IEC 14496-3 (MPEG-4 AAC LATM)

/// Extract ADTS elementary stream from MPEG-TS data.
///
/// Returns the raw ADTS bytes stripped of all TS/PES framing, suitable for
/// direct consumption by symphonia's ADTS format reader.
pub fn extract_adts_from_ts(ts_data: &[u8]) -> Vec<u8> {
  let mut adts_out = Vec::with_capacity(ts_data.len() / 2);
  let mut pmt_pid: Option<u16> = None;
  let mut audio_pid: Option<u16> = None;

  let mut offset = 0;

  // Try to find the sync byte at start
  while offset < ts_data.len() && ts_data[offset] != TS_SYNC_BYTE {
    offset += 1;
  }

  while offset + TS_PACKET_SIZE <= ts_data.len() {
    let packet = &ts_data[offset..offset + TS_PACKET_SIZE];
    offset += TS_PACKET_SIZE;

    if packet[0] != TS_SYNC_BYTE {
      // Lost sync - try to re-sync
      let remaining = &ts_data[offset..];
      if let Some(sync_pos) = remaining.iter().position(|&b| b == TS_SYNC_BYTE) {
        offset += sync_pos;
      } else {
        break;
      }
      continue;
    }

    let _transport_error = (packet[1] & 0x80) != 0;
    let payload_start = (packet[1] & 0x40) != 0;
    let pid = ((packet[1] as u16 & 0x1F) << 8) | packet[2] as u16;
    let adaptation_field_control = (packet[3] >> 4) & 0x03;

    if _transport_error {
      continue;
    }

    // Determine payload start position
    let mut payload_offset: usize = 4;

    if adaptation_field_control == 2 || adaptation_field_control == 3 {
      // Adaptation field present
      if payload_offset < TS_PACKET_SIZE {
        let adaptation_length = packet[payload_offset] as usize;
        payload_offset += 1 + adaptation_length;
      }
    }

    if adaptation_field_control == 0 || adaptation_field_control == 2 {
      // No payload
      continue;
    }

    if payload_offset >= TS_PACKET_SIZE {
      continue;
    }

    let payload = &packet[payload_offset..];

    // PAT (Program Association Table)
    if pid == PAT_PID {
      if let Some(pid) = parse_pat(payload, payload_start) {
        pmt_pid = Some(pid);
      }
      continue;
    }

    // PMT (Program Map Table)
    if Some(pid) == pmt_pid {
      if let Some(pid) = parse_pmt(payload, payload_start) {
        audio_pid = Some(pid);
      }
      continue;
    }

    // Audio PES data
    if Some(pid) == audio_pid {
      extract_pes_payload(payload, payload_start, &mut adts_out);
    }
  }

  adts_out
}

/// Parse PAT to find PMT PID.
fn parse_pat(payload: &[u8], payload_start: bool) -> Option<u16> {
  let data = if payload_start && !payload.is_empty() {
    // Skip pointer field
    let pointer = payload[0] as usize;
    if 1 + pointer >= payload.len() {
      return None;
    }
    &payload[1 + pointer..]
  } else {
    payload
  };

  // PAT header: table_id(1) + section_syntax(2) + transport_stream_id(2) +
  // version/current(1) + section_number(1) + last_section(1) = 8 bytes
  if data.len() < 8 {
    return None;
  }

  let _table_id = data[0]; // Should be 0x00 for PAT
  let section_length = ((data[1] as usize & 0x0F) << 8) | data[2] as usize;
  let header_size = 8;

  // Each program entry: program_number(2) + PMT_PID(2) = 4 bytes
  // Subtract 4 bytes CRC at end
  let entries_end = std::cmp::min(header_size + section_length.saturating_sub(5), data.len());

  let mut pos = header_size;
  while pos + 4 <= entries_end {
    let program_number = ((data[pos] as u16) << 8) | data[pos + 1] as u16;
    let pid = ((data[pos + 2] as u16 & 0x1F) << 8) | data[pos + 3] as u16;

    if program_number != 0 {
      // Non-zero = actual program, return its PMT PID
      return Some(pid);
    }
    pos += 4;
  }

  None
}

/// Parse PMT to find audio elementary stream PID.
fn parse_pmt(payload: &[u8], payload_start: bool) -> Option<u16> {
  let data = if payload_start && !payload.is_empty() {
    let pointer = payload[0] as usize;
    if 1 + pointer >= payload.len() {
      return None;
    }
    &payload[1 + pointer..]
  } else {
    payload
  };

  // PMT header: table_id(1) + section_syntax(2) + program_number(2) +
  // version/current(1) + section_number(1) + last_section(1) + PCR_PID(2) +
  // program_info_length(2) = 12 bytes
  if data.len() < 12 {
    return None;
  }

  let section_length = ((data[1] as usize & 0x0F) << 8) | data[2] as usize;
  let program_info_length = ((data[10] as usize & 0x0F) << 8) | data[11] as usize;

  let mut pos = 12 + program_info_length;
  let section_end = std::cmp::min(3 + section_length.saturating_sub(4), data.len());

  while pos + 5 <= section_end {
    let stream_type = data[pos];
    let elementary_pid = ((data[pos + 1] as u16 & 0x1F) << 8) | data[pos + 2] as u16;
    let es_info_length = ((data[pos + 3] as usize & 0x0F) << 8) | data[pos + 4] as usize;

    if stream_type == STREAM_TYPE_AAC || stream_type == STREAM_TYPE_AAC_LATM {
      return Some(elementary_pid);
    }

    // Also check for common MPEG audio (MP3 in TS)
    if stream_type == 0x03 || stream_type == 0x04 {
      return Some(elementary_pid);
    }

    pos += 5 + es_info_length;
  }

  None
}

/// Extract elementary stream payload from PES packet data.
fn extract_pes_payload(payload: &[u8], payload_start: bool, out: &mut Vec<u8>) {
  if payload_start {
    // PES packet starts here — strip PES header
    if payload.len() < 9 {
      return;
    }

    // Verify PES start code: 0x00 0x00 0x01
    if payload[0] != 0x00 || payload[1] != 0x00 || payload[2] != 0x01 {
      // Not a valid PES start — just append raw data
      out.extend_from_slice(payload);
      return;
    }

    // PES header: start_code(3) + stream_id(1) + pes_length(2) + flags(2) + header_data_length(1)
    let header_data_length = payload[8] as usize;
    let pes_header_size = 9 + header_data_length;

    if pes_header_size < payload.len() {
      out.extend_from_slice(&payload[pes_header_size..]);
    }
  } else {
    // Continuation of PES payload — append directly
    out.extend_from_slice(payload);
  }
}
