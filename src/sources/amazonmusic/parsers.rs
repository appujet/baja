use serde_json::Value;

use super::api::{clean_image_url, duration_str_to_ms, clean_song_title, normalize_artist};
use crate::protocol::tracks::TrackInfo;

pub fn parse_track(resp: &Value, track_id: &str) -> Option<TrackInfo> {
    let methods = resp["methods"].as_array()?;
    let template = &methods.first()?["template"];

    if template.is_null() {
        return None;
    }

    let widgets = template["widgets"].as_array()?;

    let tracklist = widgets.iter().find(|w| {
        w["header"]
            .as_str()
            .map(|h| h.to_lowercase().contains("album tracklist"))
            .unwrap_or(false)
    })?;

    let items = tracklist["items"].as_array()?;

    let track_item = items.iter().find(|item| {
        item["primaryTextLink"]["deeplink"]
            .as_str()
            .map(|dl| dl.contains(&format!("/tracks/{track_id}")))
            .unwrap_or(false)
    })?;

    let title = template["headerText"]["text"]
        .as_str()
        .unwrap_or("Unknown Title")
        .to_string();
    let artist = normalize_artist(
        template["headerPrimaryText"]
            .as_str()
            .unwrap_or("Unknown Artist"),
    );
    let duration_ms = duration_str_to_ms(track_item["secondaryText3"].as_str().unwrap_or(""));
    let artwork_url = template["headerImage"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(clean_image_url);
    let isrc = extract_isrc(template);

    Some(TrackInfo {
        identifier: track_id.to_string(),
        is_seekable: true,
        author: artist,
        length: duration_ms,
        is_stream: false,
        position: 0,
        title,
        uri: Some(format!("https://music.amazon.com/tracks/{track_id}")),
        artwork_url,
        isrc,
        source_name: "amazonmusic".to_string(),
    })
}

pub fn parse_album_tracks(resp: &Value, _album_id: &str) -> Option<(String, String, Vec<TrackInfo>)> {
    let template = &resp["methods"][0]["template"];
    let album_name = template["headerText"]["text"]
        .as_str()
        .unwrap_or("Unknown Album")
        .to_string();
    let artist_name = template["headerPrimaryText"]
        .as_str()
        .unwrap_or("Unknown Artist")
        .to_string();
    let artwork = template["headerImage"].as_str().unwrap_or("").to_string();

    let items = template["widgets"][0]["items"]
        .as_array()
        .cloned()
        .unwrap_or_default();

    let tracks = items
        .iter()
        .filter_map(|item| {
            let track_id = item["primaryTextLink"]["deeplink"]
                .as_str()
                .and_then(|dl| dl.split("/tracks/").nth(1))?
                .to_string();

            let title = item["primaryText"].as_str().unwrap_or("Unknown Title").to_string();
            let item_artist = normalize_artist(
                item["secondaryText2"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .unwrap_or(&artist_name),
            );
            let duration_ms = duration_str_to_ms(item["secondaryText3"].as_str().unwrap_or(""));
            let art = if artwork.is_empty() { None } else { Some(clean_image_url(&artwork)) };

            Some(TrackInfo {
                identifier: track_id.clone(),
                is_seekable: true,
                author: item_artist,
                length: duration_ms,
                is_stream: false,
                position: 0,
                title,
                uri: Some(format!(
                    "https://music.amazon.com/tracks/{track_id}"
                )),
                artwork_url: art,
                isrc: None,
                source_name: "amazonmusic".to_string(),
            })
        })
        .collect();

    Some((album_name, artist_name, tracks))
}

pub struct ArtistResult {
    pub name: String,
    pub artwork_url: Option<String>,
    pub tracks: Vec<TrackInfo>,
}

pub fn parse_artist_top_songs(
    resp: &Value,
    artist_id: &str,
    duration_map: &std::collections::HashMap<String, u64>,
) -> Option<ArtistResult> {
    let template = &resp["methods"][0]["template"];
    let artist_name = template["headerText"]["text"]
        .as_str()
        .unwrap_or("Unknown Artist")
        .to_string();
    let artwork_url = template["backgroundImage"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(clean_image_url);

    let widgets = template["widgets"].as_array()?;
    let top_songs_widget = widgets.iter().find(|w| {
        w["header"]
            .as_str()
            .map(|h| h.to_lowercase().contains("top songs"))
            .unwrap_or(false)
    })?;

    let items = top_songs_widget["items"].as_array()?;

    let tracks = items
        .iter()
        .filter_map(|item| {
            let storage_key = item["iconButton"]["observer"]["storageKey"].as_str()?;
            let mut parts = storage_key.splitn(2, ':');
            let album_id = parts.next()?.to_string();
            let track_id = parts.next()?.to_string();
            if track_id.is_empty() {
                return None;
            }

            let title = clean_song_title(item["primaryText"]["text"].as_str().unwrap_or("Unknown Title"));
            let artist = normalize_artist(
                item["secondaryText"].as_str().unwrap_or(&artist_name),
            );
            let item_artwork = item["image"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(clean_image_url)
                .or_else(|| artwork_url.clone());

            let duration_ms = duration_map
                .get(&format!("{album_id}:{track_id}"))
                .copied()
                .unwrap_or(0);

            Some(TrackInfo {
                identifier: track_id.clone(),
                is_seekable: true,
                author: artist,
                length: duration_ms,
                is_stream: false,
                position: 0,
                title,
                uri: Some(format!("https://music.amazon.com/tracks/{track_id}")),
                artwork_url: item_artwork,
                isrc: None,
                source_name: "amazonmusic".to_string(),
            })
        })
        .collect();

    let _ = artist_id;

    Some(ArtistResult { name: artist_name, artwork_url, tracks })
}

pub struct PlaylistResult {
    pub name: String,
    pub artwork_url: Option<String>,
    pub tracks: Vec<TrackInfo>,
}

pub fn parse_playlist_tracks(resp: &Value) -> Option<PlaylistResult> {
    let template = &resp["methods"][0]["template"];
    let name = template["headerText"]["text"]
        .as_str()
        .unwrap_or("Unknown Playlist")
        .to_string();
    let artwork_url = template["headerImage"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(clean_image_url);

    let items = template["widgets"][0]["items"]
        .as_array()
        .cloned()
        .unwrap_or_default();

    let tracks = items
        .iter()
        .filter_map(|item| {
            let storage_key = item["iconButton"]["observer"]["storageKey"].as_str()?;
            let mut parts = storage_key.splitn(2, ':');
            let _album_id = parts.next()?;
            let track_id = parts.next()?.to_string();
            if track_id.is_empty() {
                return None;
            }

            let title = item["primaryText"].as_str().unwrap_or("Unknown Title").to_string();
            let artist = normalize_artist(
                item["secondaryText1"].as_str().unwrap_or("Unknown Artist"),
            );
            let duration_ms = duration_str_to_ms(item["secondaryText3"].as_str().unwrap_or(""));
            let item_art = item["image"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(clean_image_url)
                .or_else(|| artwork_url.clone());

            Some(TrackInfo {
                identifier: track_id.clone(),
                is_seekable: true,
                author: artist,
                length: duration_ms,
                is_stream: false,
                position: 0,
                title,
                uri: Some(format!("https://music.amazon.com/tracks/{track_id}")),
                artwork_url: item_art,
                isrc: None,
                source_name: "amazonmusic".to_string(),
            })
        })
        .collect();

    Some(PlaylistResult { name, artwork_url, tracks })
}

pub fn parse_community_playlist_tracks(resp: &Value) -> Option<PlaylistResult> {
    let template = &resp["methods"][0]["template"];
    let name = template["headerText"]["text"]
        .as_str()
        .unwrap_or("Unknown Playlist")
        .to_string();
    let artwork_url = template["headerImage"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(clean_image_url);

    let items = template["widgets"][0]["items"]
        .as_array()
        .cloned()
        .unwrap_or_default();

    let tracks = items
        .iter()
        .filter_map(|item| {
            let track_id = item["id"].as_str()?.to_string();
            if track_id.is_empty() {
                return None;
            }

            let title = item["primaryText"].as_str().unwrap_or("Unknown Title").to_string();
            let artist = normalize_artist(
                item["secondaryText1"].as_str().unwrap_or("Unknown Artist"),
            );
            let duration_ms = duration_str_to_ms(item["secondaryText3"].as_str().unwrap_or(""));
            let item_art = item["image"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(clean_image_url)
                .or_else(|| artwork_url.clone());

            Some(TrackInfo {
                identifier: track_id.clone(),
                is_seekable: true,
                author: artist,
                length: duration_ms,
                is_stream: false,
                position: 0,
                title,
                uri: Some(format!("https://music.amazon.com/tracks/{track_id}")),
                artwork_url: item_art,
                isrc: None,
                source_name: "amazonmusic".to_string(),
            })
        })
        .collect();

    Some(PlaylistResult { name, artwork_url, tracks })
}

pub fn parse_search_tracks(
    resp: &Value,
    limit: usize,
    duration_map: &std::collections::HashMap<String, u64>,
) -> Vec<TrackInfo> {
    let items = match resp["methods"]
        .as_array()
        .and_then(|m| m.first())
        .and_then(|m| m["template"]["widgets"].as_array())
        .and_then(|w| w.first())
        .and_then(|w| w["items"].as_array())
    {
        Some(i) => i,
        None => return Vec::new(),
    };

    items
        .iter()
        .take(limit)
        .filter_map(|item| {
            let storage_key = item["iconButton"]["observer"]["storageKey"].as_str()?;
            let mut parts = storage_key.splitn(2, ':');
            let album_id = parts.next()?.to_string();
            let track_id = parts.next()?.to_string();
            if track_id.is_empty() {
                return None;
            }

            let title = item["primaryText"]["text"]
                .as_str()
                .unwrap_or("Unknown Title")
                .to_string();

            let artist = normalize_artist(
                item["secondaryText"].as_str().unwrap_or("Unknown Artist"),
            );

            let artwork = item["image"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(clean_image_url);

            let duration_ms = duration_map
                .get(&format!("{album_id}:{track_id}"))
                .copied()
                .unwrap_or(0);

            Some(TrackInfo {
                identifier: track_id.clone(),
                is_seekable: true,
                author: artist,
                length: duration_ms,
                is_stream: false,
                position: 0,
                title,
                uri: Some(format!("https://music.amazon.com/tracks/{track_id}")),
                artwork_url: artwork,
                isrc: None,
                source_name: "amazonmusic".to_string(),
            })
        })
        .collect()
}

fn extract_isrc(template: &Value) -> Option<String> {
    let scripts = template["templateData"]["seoHead"]["script"].as_array()?;
    for script in scripts {
        let inner_html = script["innerHTML"].as_str()?;
        if let Ok(parsed) = serde_json::from_str::<Value>(inner_html) {
            if let Some(isrc) = parsed["isrcCode"].as_str() {
                return Some(isrc.to_string());
            }
        }
    }
    None
}
