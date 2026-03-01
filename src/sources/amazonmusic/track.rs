use serde_json::Value;
use regex::Regex;
use crate::protocol::tracks::{LoadResult, TrackInfo, Track, PlaylistData, PlaylistInfo};
use super::utils::*;

pub async fn fallback_to_odesli(client: &reqwest::Client, url: &str, target_id: Option<&str>) -> LoadResult {
    let api_url = format!("https://api.song.link/v1-alpha.1/links?url={}", urlencoding::encode(url.split('?').next().unwrap()));
    if let Ok(res) = client.get(&api_url).send().await {
        if res.status().is_success() {
            if let Ok(body) = res.json::<Value>().await {
                let unique_id = body["entityUniqueId"].as_str().unwrap_or("");
                let mut entity = &body["entitiesByUniqueId"][unique_id];
                
                if let Some(tid) = target_id {
                    if !entity["id"].as_str().map_or(false, |id| id.contains(tid)) {
                        if let Some(obj) = body["entitiesByUniqueId"].as_object() {
                            if let Some(found) = obj.values().find(|e| e["id"].as_str().map_or(false, |id| id.contains(tid))) {
                                entity = found;
                            }
                        }
                    }
                }

                if !entity.is_null() {
                    let track_info = TrackInfo {
                        identifier: entity["id"].as_str().unwrap_or("unknown").to_string(),
                        is_seekable: true,
                        author: entity["artistName"].as_str().unwrap_or("Unknown Artist").to_string(),
                        length: 0,
                        is_stream: false,
                        position: 0,
                        title: entity["title"].as_str().unwrap_or("Unknown Track").to_string(),
                        uri: Some(url.to_string()),
                        artwork_url: entity["thumbnailUrl"].as_str().map(|s| s.to_string()),
                        isrc: entity["isrc"].as_str().map(|s| s.to_string()),
                        source_name: "amazonmusic".to_string(),
                    };

                    return LoadResult::Track(Track::new(track_info));
                }
            }
        }
    }
    LoadResult::Empty {}
}

pub async fn fetch_json_ld(
    client: &reqwest::Client,
    url: &str,
    target_id: Option<&str>
) -> Option<LoadResult> {
    const BOT_UA: &str = "Mozilla/5.0 (compatible; NodeLinkBot/0.1; +https://nodelink.js.org/)";
    let res = tokio::time::timeout(std::time::Duration::from_secs(4), client.get(url)
        .header("User-Agent", BOT_UA)
        .send()).await.ok()?.ok()?;

    if !res.status().is_success() {
        return None;
    }

    let body = res.text().await.ok()?;
    
    let re_header_title = Regex::new(r#"<music-detail-header[^>]*primary-text="([^"]+)""#).unwrap();
    let header_title = re_header_title.captures(&body)
        .map(|c| decode_amp(c.get(1).unwrap().as_str()));

    let re_header_author = Regex::new(r#"<music-detail-header[^>]*secondary-text="([^"]+)""#).unwrap();
    let header_author = re_header_author.captures(&body)
        .map(|c| decode_amp(c.get(1).unwrap().as_str()));

    let re_header_image = Regex::new(r#"<music-detail-header[^>]*image-src="([^"]+)""#).unwrap();
    let header_image = re_header_image.captures(&body)
        .map(|c| c.get(1).unwrap().as_str().to_string());

    let re_og_image = Regex::new(r#"<meta property="og:image" content="([^"]+)""#).unwrap();
    let og_image = re_og_image.captures(&body)
        .map(|c| c.get(1).unwrap().as_str().to_string());

    let artwork_url = header_image.or(og_image);

    let re_json_ld = Regex::new(r#"(?s)<script [^>]*type="application/ld\+json"[^>]*>(.*?)</script>"#).unwrap();
    let mut collection = None;
    let mut track_data = None;

    for caps in re_json_ld.captures_iter(&body) {
        let content = caps.get(1).unwrap().as_str().replace("&quot;", "\"").replace("&amp;", "&");
        if let Ok(parsed) = serde_json::from_str::<Value>(&content) {
            let data = if parsed.is_array() { &parsed[0] } else { &parsed };
            let type_str = data["@type"].as_str().unwrap_or("");
            if type_str == "MusicAlbum" || type_str == "MusicGroup" || type_str == "Playlist" {
                collection = Some(data.clone());
            } else if type_str == "MusicRecording" {
                track_data = Some(data.clone());
            }
        }
    }

    let mut tracks = Vec::new();
    let mut playlist_name = header_title.clone().unwrap_or_else(|| "Unknown".to_string());
    let mut collection_author = header_author.clone().unwrap_or_else(|| "Unknown Artist".to_string());
    let mut collection_image = artwork_url.clone();

    if let Some(ref col) = collection {
        let type_str = col["@type"].as_str().unwrap_or("");
        
        // If the LD contains a name, it's either the Playlist name, Album name, or Artist name (if MusicGroup)
        if let Some(n) = col["name"].as_str() {
            playlist_name = decode_amp(n);
        }

        let extracted_artist = col["byArtist"]["name"].as_str()
            .or_else(|| col["byArtist"][0]["name"].as_str())
            .or_else(|| col["author"]["name"].as_str())
            .or_else(|| {
                // If this is an artist page natively, the author is the group's name
                if type_str == "MusicGroup" {
                    col["name"].as_str()
                } else {
                    None
                }
            });
        
        if let Some(name) = extracted_artist {
            collection_author = decode_amp(name);
        }

        if let Some(img) = col["image"].as_str() {
            collection_image = Some(img.to_string());
        }

        if let Some(col_tracks) = col["track"].as_array() {
            for t in col_tracks {
                let id = t["url"].as_str().and_then(|u| u.split('/').last())
                    .or_else(|| t["@id"].as_str().and_then(|u| u.split('/').last()))
                    .unwrap_or_else(|| "unknown")
                    .to_string();

                let raw_author = t["byArtist"]["name"].as_str()
                    .or(t["byArtist"][0]["name"].as_str())
                    .or(t["author"]["name"].as_str());
                
                let track_author = raw_author.map(|a| decode_amp(a)).unwrap_or_else(|| collection_author.clone());

                tracks.push(TrackInfo {
                    identifier: id,
                    is_seekable: true,
                    author: track_author,
                    length: parse_iso8601_duration(t["duration"].as_str().unwrap_or("")),
                    is_stream: false,
                    position: 0,
                    title: decode_amp(t["name"].as_str().unwrap_or("Unknown Track")),
                    uri: Some(t["url"].as_str().unwrap_or(url).to_string()),
                    artwork_url: collection_image.clone(),
                    isrc: t["isrcCode"].as_str().map(|s| s.to_string()),
                    source_name: "amazonmusic".to_string(),
                });
            }
        }
    }

    if tracks.is_empty() {
        let re_row = Regex::new(r#"(?s)<(?:music-image-row|music-text-row)[^>]*primary-text="([^"]+)"[^>]*primary-href="([^"]+)"(?:[^>]*secondary-text-1="([^"]+)")?[^>]*duration="([^"]+)"(?:[^>]*image-src="([^"]+)")?"#).unwrap();
        for caps in re_row.captures_iter(&body) {
            let t_title = decode_amp(caps.get(1).unwrap().as_str());
            let t_href = caps.get(2).unwrap().as_str();
            let t_artist = caps.get(3).map(|m| decode_amp(m.as_str())).unwrap_or_else(|| collection_author.clone());
            let t_duration = caps.get(4).unwrap().as_str();
            let t_image = caps.get(5).map(|m| m.as_str().to_string()).or(collection_image.clone());
            
            if let Some(t_id) = extract_identifier(t_href) {
                tracks.push(TrackInfo {
                    identifier: t_id.clone(),
                    is_seekable: true,
                    author: t_artist,
                    length: if t_duration.contains(':') { parse_colon_duration_to_ms(t_duration) } else { 0 },
                    is_stream: false,
                    position: 0,
                    title: t_title,
                    uri: Some(format!("https://music.amazon.com/tracks/{}", t_id)),
                    artwork_url: t_image,
                    isrc: None,
                    source_name: "amazonmusic".to_string(),
                });
            }
        }
    }

    if !tracks.is_empty() {
        if let Some(tid) = target_id {
            if let Some(selected) = tracks.iter().find(|t| t.identifier == tid || t.uri.as_ref().map_or(false, |u| u.contains(tid))) {
                return Some(LoadResult::Track(Track::new(selected.clone())));
            }
        }

        if url.contains("/tracks/") && target_id.is_none() {
            return Some(LoadResult::Track(Track::new(tracks[0].clone())));
        }

        return Some(LoadResult::Playlist(PlaylistData {
            info: PlaylistInfo {
                name: playlist_name,
                selected_track: 0,
            },
            plugin_info: serde_json::json!({}),
            tracks: tracks.into_iter().map(Track::new).collect(),
        }));
    }

    if let Some(td) = track_data {
        let artist = td["byArtist"]["name"].as_str()
            .or_else(|| td["byArtist"][0]["name"].as_str())
            .or_else(|| td["author"]["name"].as_str())
            .or_else(|| td["author"][0]["name"].as_str())
            .unwrap_or("Unknown Artist");
        let track_image = td["image"].as_str().map(|s| s.to_string()).or(artwork_url);
        
        let track_info = TrackInfo {
            identifier: td["id"].as_str().or(td["isrcCode"].as_str()).or(url.split('/').last()).unwrap_or("unknown").to_string(),
            is_seekable: true,
            author: decode_amp(artist),
            length: parse_iso8601_duration(td["duration"].as_str().unwrap_or("")),
            is_stream: false,
            position: 0,
            title: decode_amp(td["name"].as_str().unwrap_or("Unknown Track")),
            uri: Some(url.to_string()),
            artwork_url: track_image,
            isrc: td["isrcCode"].as_str().map(|s| s.to_string()),
            source_name: "amazonmusic".to_string(),
        };

        return Some(LoadResult::Track(Track::new(track_info)));
    }

    None
}
