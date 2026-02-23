use std::collections::HashMap;

use super::{
    types::{ByteRange, M3u8Playlist, Media, Resource, Variant},
    utils::{extract_attr_str, extract_attr_u64, parse_byte_range, resolve_url},
};

/// Very small M3U8 parser — handles just enough of the spec for YouTube HLS.
pub fn parse_m3u8(text: &str, base_url: &str) -> M3u8Playlist {
    let lines: Vec<&str> = text.lines().map(str::trim).collect();

    // Decide master vs. media by presence of EXT-X-STREAM-INF.
    let is_master = lines.iter().any(|l| l.starts_with("#EXT-X-STREAM-INF"));

    if is_master {
        let mut variants = Vec::new();
        let mut audio_groups: HashMap<String, Vec<Media>> = HashMap::new();
        let mut i = 0;

        while i < lines.len() {
            let line = lines[i];
            if line.starts_with("#EXT-X-MEDIA") {
                let type_ = extract_attr_str(line, "TYPE").unwrap_or_default();
                let group_id = extract_attr_str(line, "GROUP-ID").unwrap_or_default();
                let uri = extract_attr_str(line, "URI").map(|u| resolve_url(base_url, &u));
                let is_default = extract_attr_str(line, "DEFAULT").as_deref() == Some("YES");

                if type_ == "AUDIO" && !group_id.is_empty() {
                    audio_groups
                        .entry(group_id.clone())
                        .or_default()
                        .push(Media {
                            _type: type_,
                            _group_id: group_id,
                            uri,
                            is_default,
                        });
                }
                i += 1;
            } else if line.starts_with("#EXT-X-STREAM-INF") {
                let bandwidth = extract_attr_u64(line, "BANDWIDTH").unwrap_or(0);
                let codecs = extract_attr_str(line, "CODECS").unwrap_or_default();
                let audio_group = extract_attr_str(line, "AUDIO");

                let has_audio =
                    codecs.contains("mp4a") || codecs.contains("opus") || codecs.contains("aac");
                let has_video = codecs.contains("avc1")
                    || codecs.contains("hvc1")
                    || codecs.contains("hev1")
                    || codecs.contains("dvh1")
                    || codecs.contains("vp09")
                    || codecs.contains("av01")
                    || codecs.contains("vp9")
                    || codecs.contains("av1")
                    || codecs.contains("vp8")
                    || codecs.contains("h264")
                    || codecs.contains("h265")
                    || codecs.contains("mp4v");

                let mut j = i + 1;
                while j < lines.len() && lines[j].starts_with('#') {
                    j += 1;
                }
                if j < lines.len() && !lines[j].is_empty() {
                    variants.push(Variant {
                        url: resolve_url(base_url, lines[j]),
                        bandwidth,
                        codecs: codecs.clone(),
                        is_audio_only: has_audio && !has_video,
                        audio_group,
                    });
                }
                i = j + 1;
            } else {
                i += 1;
            }
        }

        return M3u8Playlist::Master {
            variants,
            audio_groups,
        };
    }

    let mut segments = Vec::new();
    let mut map = None;
    let mut next_offset = 0u64;
    let mut pending_range: Option<ByteRange> = None;

    for i in 0..lines.len() {
        let line = lines[i];
        if line.starts_with("#EXT-X-MAP") {
            if let Some(url) = extract_attr_str(line, "URI").map(|u| resolve_url(base_url, &u)) {
                let range = extract_attr_str(line, "BYTERANGE").map(|r| parse_byte_range(&r, 0));
                map = Some(Resource {
                    url,
                    range,
                    duration: None,
                });
            }
        } else if line.starts_with("#EXT-X-BYTERANGE:") {
            let r = parse_byte_range(&line[17..], next_offset);
            next_offset = r.offset + r.length;
            pending_range = Some(r);
        } else if line.starts_with("#EXTINF:") {
            // Parse duration from #EXTINF:<duration>,
            let seg_duration = line
                .strip_prefix("#EXTINF:")
                .and_then(|rest| rest.split(',').next())
                .and_then(|d| d.trim().parse::<f64>().ok());

            let mut j = i + 1;
            while j < lines.len() && lines[j].starts_with('#') {
                if lines[j].starts_with("#EXT-X-BYTERANGE:") {
                    let r = parse_byte_range(&lines[j][17..], next_offset);
                    next_offset = r.offset + r.length;
                    pending_range = Some(r);
                }
                j += 1;
            }
            if j < lines.len() {
                segments.push(Resource {
                    url: resolve_url(base_url, lines[j]),
                    range: pending_range.take(),
                    duration: seg_duration,
                });
            }
        }
    }
    M3u8Playlist::Media { segments, map }
}
