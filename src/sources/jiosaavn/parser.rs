use serde_json::Value;

use super::helpers::clean_string;
use crate::protocol::tracks::{PlaylistData, PlaylistInfo, Track, TrackInfo};

pub fn parse_track(json: &Value) -> Option<Track> {
    let id = json.get("id").and_then(|v| {
        v.as_str()
            .map(|s| s.to_string())
            .or_else(|| v.as_i64().map(|i| i.to_string()))
    })?;

    let title_raw = json.get("title").or_else(|| json.get("song"))?.as_str()?;
    let title = clean_string(title_raw);

    let uri = json
        .get("perma_url")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let duration_str = json
        .get("more_info")
        .and_then(|m| m.get("duration"))
        .or_else(|| json.get("duration"))
        .and_then(|v| v.as_str())
        .unwrap_or("0");
    let duration = duration_str.parse::<u64>().unwrap_or(0) * 1000;

    let primary_artists = json
        .get("more_info")
        .and_then(|m| m.get("artistMap"))
        .and_then(|am| am.get("primary_artists"));
    let artists = json
        .get("more_info")
        .and_then(|m| m.get("artistMap"))
        .and_then(|am| am.get("artists"));

    let meta_artists = if let Some(arr) = primary_artists.and_then(|v| v.as_array()) {
        if !arr.is_empty() {
            Some(arr)
        } else {
            artists.and_then(|v| v.as_array())
        }
    } else {
        artists.and_then(|v| v.as_array())
    };

    let author = if let Some(arr) = meta_artists {
        arr.iter()
            .filter_map(|a| a.get("name").and_then(|v| v.as_str()))
            .collect::<Vec<_>>()
            .join(", ")
    } else {
        json.get("more_info")
            .and_then(|m| m.get("music"))
            .or_else(|| json.get("primary_artists"))
            .or_else(|| json.get("singers"))
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown Artist")
            .to_string()
    };
    let author = clean_string(&author);

    let artwork_url = json
        .get("image")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.replace("150x150", "500x500").replace("50x50", "500x500"));

    let track_info = TrackInfo {
        identifier: id,
        is_seekable: true,
        author,
        length: duration,
        is_stream: false,
        position: 0,
        title,
        uri,
        artwork_url,
        isrc: None,
        source_name: "jiosaavn".to_string(),
    };

    Some(Track::new(track_info))
}

pub fn parse_search_item(json: &Value) -> Option<Track> {
    let id = json.get("id")?.as_str()?;
    let title = clean_string(json.get("title")?.as_str()?);
    let author = clean_string(
        json.get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown Artist"),
    );
    let uri = json
        .get("url")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let artwork_url = json
        .get("image")
        .and_then(|v| v.as_str())
        .map(|s| s.replace("150x150", "500x500").replace("50x50", "500x500"));

    let track_info = TrackInfo {
        identifier: id.to_string(),
        is_seekable: true,
        author,
        length: 0,
        is_stream: false,
        position: 0,
        title,
        uri,
        artwork_url,
        isrc: None,
        source_name: "jiosaavn".to_string(),
    };

    Some(Track::new(track_info))
}

pub fn parse_search_playlist(json: &Value, type_: &str) -> Option<PlaylistData> {
    let title = clean_string(json.get("title")?.as_str()?);
    let mut url = json
        .get("url")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    if url.is_empty() {
        url = json
            .get("perma_url")
            .or_else(|| json.get("permaurl"))
            .or_else(|| json.get("token"))
            .or_else(|| json.pointer("/more_info/perma_url"))
            .or_else(|| json.pointer("/more_info/token"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
    }

    if url.is_empty() {
        if let Some(id) = json.get("id").and_then(|v| {
            v.as_str()
                .map(|s| s.to_string())
                .or_else(|| v.as_i64().map(|i| i.to_string()))
        }) {
            if id.starts_with("/") || id.starts_with("http") {
                url = id;
            } else {
                // Fallback to constructing a URL from token/id
                let path_type = match type_ {
                    "playlist" => "s/playlist",
                    "featured" => "featured",
                    "album" => "album",
                    "artist" => "artist",
                    _ => type_,
                };
                let slug = title
                    .to_lowercase()
                    .replace(|c: char| !c.is_alphanumeric() && c != ' ', "")
                    .replace(' ', "-");
                url = format!("/{}/{}/{}", path_type, slug, id);
            }
        }
    }

    if !url.is_empty() && !url.starts_with("http") {
        url = format!("https://www.jiosaavn.com{}", url);
    }

    let artwork_url = json
        .get("image")
        .and_then(|v| v.as_str())
        .map(|s| s.replace("150x150", "500x500").replace("50x50", "500x500"));

    let total_tracks = json
        .get("more_info")
        .and_then(|m| m.get("song_count").or_else(|| m.get("track_count")))
        .and_then(|v| {
            v.as_str()
                .and_then(|s| s.parse::<u64>().ok())
                .or_else(|| v.as_u64())
        })
        .or_else(|| {
            json.get("song_count")
                .or_else(|| json.get("track_count"))
                .and_then(|v| {
                    v.as_str()
                        .and_then(|s| s.parse::<u64>().ok())
                        .or_else(|| v.as_u64())
                })
        })
        .or_else(|| {
            json.pointer("/more_info/song_pids")
                .and_then(|v| v.as_str())
                .map(|s| {
                    if s.is_empty() {
                        0
                    } else {
                        s.split(',').count() as u64
                    }
                })
        })
        .unwrap_or(0);

    let mut author = json
        .pointer("/more_info/artist_name")
        .or_else(|| json.pointer("/more_info/music"))
        .or_else(|| json.get("music"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    if author.is_none() {
        let first = json
            .pointer("/more_info/firstname")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty());
        let last = json
            .pointer("/more_info/lastname")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty());
        if let Some(f) = first {
            author = Some(format!("{} {}", f, last.unwrap_or("")).trim().to_string());
        } else if let Some(l) = last {
            author = Some(l.to_string());
        }
    }

    if author.is_none() {
        author = json
            .get("subtitle")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .or_else(|| {
                json.get("description")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
            })
            .map(|s| s.to_string());
    }

    let final_author = if type_ == "artist" {
        title.clone()
    } else {
        author.unwrap_or_else(|| "Unknown Author".to_string())
    };

    Some(PlaylistData {
        info: PlaylistInfo {
            name: title,
            selected_track: -1,
        },
        plugin_info: serde_json::json!({
            "url": url,
            "type": type_,
            "artworkUrl": artwork_url,
            "author": clean_string(&final_author),
            "totalTracks": total_tracks
        }),
        tracks: Vec::new(),
    })
}
