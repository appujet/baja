use crate::api::tracks::{Track, TrackInfo};
use serde_json::Value;

pub fn extract_from_player(body: &Value, source_name: &str) -> Option<Track> {
    let details = body.get("videoDetails")?;
    let video_id = details.get("videoId")?.as_str()?;

    let title = details.get("title")?.as_str()?.to_string();
    let author = details.get("author")?.as_str()?.to_string();
    let is_stream = details
        .get("isLiveContent")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let length_seconds = details
        .get("lengthSeconds")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);

    let artwork_url = details
        .get("thumbnail")
        .and_then(|t| t.get("thumbnails"))
        .and_then(|arr| arr.as_array())
        .and_then(|arr| arr.last())
        .and_then(|thumb| thumb.get("url"))
        .and_then(|url| url.as_str())
        .map(|s| s.to_string());

    Some(Track::new(TrackInfo {
        identifier: video_id.to_string(),
        is_seekable: !is_stream,
        author,
        length: if is_stream { 0 } else { length_seconds * 1000 },
        is_stream,
        position: 0,
        title,
        uri: Some(format!("https://www.youtube.com/watch?v={}", video_id)),
        artwork_url,
        isrc: None,
        source_name: source_name.to_string(),
    }))
}

pub fn extract_from_browse(body: &Value, source_name: &str) -> Option<(Vec<Track>, String)> {
    let title = body
        .get("header")
        .and_then(|h| {
            h.get("playlistHeaderRenderer")
                .or_else(|| h.get("musicAlbumReleaseHeaderRenderer"))
        })
        .and_then(|h| h.get("title"))
        .and_then(|t| get_text(t))
        .unwrap_or_else(|| "Unknown Playlist".to_string());

    let mut tracks = Vec::new();
    if let Some(section_list) = find_section_list(body) {
        if let Some(contents) = section_list.get("contents").and_then(|c| c.as_array()) {
            for section in contents {
                // For standard YouTube playlists
                if let Some(list) = section
                    .get("itemSectionRenderer")
                    .and_then(|i| i.get("contents"))
                    .and_then(|c| c.as_array())
                    .and_then(|arr| arr.get(0))
                    .and_then(|first| first.get("playlistVideoListRenderer"))
                    .and_then(|p| p.get("contents"))
                    .and_then(|c| c.as_array())
                {
                    for item in list {
                        if let Some(track) = extract_track(item, source_name) {
                            tracks.push(track);
                        }
                    }
                }
                // For YouTube Music shelves
                if let Some(list) = section
                    .get("musicShelfRenderer")
                    .and_then(|s| s.get("contents"))
                    .and_then(|c| c.as_array())
                {
                    for item in list {
                        if let Some(track) = extract_track(item, source_name) {
                            tracks.push(track);
                        }
                    }
                }
            }
        }
    }

    if tracks.is_empty() {
        return None;
    }

    Some((tracks, title))
}

pub fn find_section_list(value: &Value) -> Option<&Value> {
    if let Some(list) = value.get("sectionListRenderer") {
        return Some(list);
    }
    if let Some(contents) = value.get("contents") {
        if let Some(list) = find_section_list(contents) {
            return Some(list);
        }
    }
    if let Some(arr) = value.as_array() {
        for item in arr {
            if let Some(list) = find_section_list(item) {
                return Some(list);
            }
        }
    }
    if let Some(tabs) = value.get("tabs").and_then(|t| t.as_array()) {
        for tab in tabs {
            if let Some(content) = tab.get("tabRenderer").and_then(|tr| tr.get("content")) {
                if let Some(list) = find_section_list(content) {
                    return Some(list);
                }
            }
        }
    }
    if let Some(primary) = value
        .get("twoColumnSearchResultsRenderer")
        .and_then(|t| t.get("primaryContents"))
    {
        return find_section_list(primary);
    }
    None
}

pub fn extract_track(item: &Value, source_name: &str) -> Option<Track> {
    let renderer = item
        .get("videoRenderer")
        .or_else(|| item.get("compactVideoRenderer"))
        .or_else(|| item.get("playlistVideoRenderer"))
        .or_else(|| item.get("musicResponsiveListItemRenderer"))
        .or_else(|| item.get("musicTwoColumnItemRenderer"))?;

    let video_id = renderer
        .get("videoId")
        .and_then(|v| v.as_str())
        .or_else(|| {
            renderer
                .get("playlistItemData")
                .and_then(|d| d.get("videoId"))
                .and_then(|v| v.as_str())
        })
        .or_else(|| {
            renderer
                .get("doubleTapCommand")
                .and_then(|c| c.get("watchEndpoint"))
                .and_then(|w| w.get("videoId"))
                .and_then(|v| v.as_str())
        })?;

    let title = get_text(renderer.get("title").or_else(|| {
        renderer
            .get("flexColumns")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("musicResponsiveListItemFlexColumnRenderer"))
            .and_then(|r| r.get("text"))
    })?)?;

    let author = if let Some(long_byline) = renderer.get("longBylineText") {
        get_text(long_byline)
    } else if let Some(short_byline) = renderer.get("shortBylineText") {
        get_text(short_byline)
    } else if let Some(owner) = renderer.get("ownerText") {
        get_text(owner)
    } else if let Some(flex) = renderer
        .get("flexColumns")
        .and_then(|c| c.get(1))
        .and_then(|c| c.get("musicResponsiveListItemFlexColumnRenderer"))
        .and_then(|r| r.get("text"))
    {
        get_text(flex)
    } else {
        Some("Unknown Artist".to_string())
    }
    .unwrap_or("Unknown Artist".to_string());

    let is_stream = renderer
        .get("isLive")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || renderer
            .get("badges")
            .and_then(|b| b.as_array())
            .map(|arr| {
                arr.iter().any(|badge| {
                    badge
                        .get("metadataBadgeRenderer")
                        .and_then(|mbr| mbr.get("label"))
                        .and_then(|l| l.as_str())
                        .map(|s| s == "LIVE")
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false);

    let length_ms = if is_stream {
        9223372036854775807 // u64::MAX technically for duration but here we use large number
    } else {
        renderer
            .get("lengthText")
            .and_then(|t| get_text(t))
            .map(|s| parse_duration(&s))
            .unwrap_or(0)
    };

    Some(Track::new(TrackInfo {
        identifier: video_id.to_string(),
        is_seekable: !is_stream,
        author,
        length: length_ms as u64,
        is_stream,
        position: 0,
        title,
        uri: Some(format!("https://www.youtube.com/watch?v={}", video_id)),
        artwork_url: get_thumbnail(renderer),
        isrc: None,
        source_name: source_name.to_string(),
    }))
}

fn get_text(obj: &Value) -> Option<String> {
    if let Some(s) = obj.as_str() {
        return Some(s.to_string());
    }
    if let Some(simple_text) = obj.get("simpleText").and_then(|v| v.as_str()) {
        return Some(simple_text.to_string());
    }
    if let Some(runs) = obj.get("runs").and_then(|v| v.as_array()) {
        let mut text = String::new();
        for run in runs {
            if let Some(t) = run.get("text").and_then(|v| v.as_str()) {
                text.push_str(t);
            }
        }
        return Some(text);
    }
    None
}

fn parse_duration(s: &str) -> i64 {
    let parts: Vec<&str> = s.split(':').collect();
    let mut seconds = 0;
    for part in parts {
        seconds = seconds * 60 + part.parse::<i64>().unwrap_or(0);
    }
    seconds * 1000
}

fn get_thumbnail(renderer: &Value) -> Option<String> {
    renderer
        .get("thumbnail")
        .and_then(|t| t.get("thumbnails"))
        .and_then(|arr| arr.as_array())
        .and_then(|arr| arr.last()) // Get highest quality
        .and_then(|thumb| thumb.get("url"))
        .and_then(|url| url.as_str())
        .map(|s| s.split('?').next().unwrap_or(s).to_string())
}
