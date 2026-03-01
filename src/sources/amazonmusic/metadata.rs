use serde_json::Value;
use std::sync::Arc;
use crate::protocol::tracks::{LoadResult, TrackInfo, Track};
use super::token::{AmazonMusicTokenTracker, AmazonConfig};
use super::utils::*;

pub async fn fetch_track_duration_api(
    client: &reqwest::Client,
    token_tracker: &AmazonMusicTokenTracker,
    track_id: &str,
    provided_cfg: Option<Arc<AmazonConfig>>,
) -> Option<u64> {
    let cfg = match provided_cfg {
        Some(c) => (*c).clone(),
        None => token_tracker.get_config().await?,
    };
    
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_millis();
    let csrf_header = token_tracker.build_csrf_header(&cfg.csrf);

    let page_url = format!("https://music.amazon.com/tracks/{}", track_id);
    let headers_obj = build_amazon_headers(&cfg, now, &csrf_header, &page_url);

    let payload = serde_json::json!({
        "id": track_id,
        "userHash": "{\"level\":\"LIBRARY_MEMBER\"}",
        "headers": headers_obj.to_string()
    });

    // Add a 3s timeout to duration fetch to prevent hanging the search
    let res = tokio::time::timeout(std::time::Duration::from_secs(3), client.post(CATALOG_TRACK_URL)
        .header("User-Agent", SEARCH_USER_AGENT)
        .header("Content-Type", "text/plain;charset=UTF-8")
        .header("Origin", "https://music.amazon.com")
        .header("Referer", "https://music.amazon.com/")
        .body(payload.to_string())
        .send()).await.ok()?.ok()?;

    if !res.status().is_success() {
        return None;
    }

    let data: Value = res.json().await.ok()?;
    let t = data["methods"][0]["template"]["headerTertiaryText"].as_str()?;
    let duration = parse_time_string_to_ms(t);
    if duration > 0 { Some(duration) } else { None }
}

pub async fn fetch_track_info_api(
    client: &reqwest::Client,
    token_tracker: &AmazonMusicTokenTracker,
    track_id: &str,
) -> Option<LoadResult> {
    let cfg = token_tracker.get_config().await?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_millis();
    let csrf_header = token_tracker.build_csrf_header(&cfg.csrf);

    let page_url = format!("https://music.amazon.com/tracks/{}", track_id);
    let headers_obj = build_amazon_headers(&cfg, now, &csrf_header, &page_url);

    let payload = serde_json::json!({
        "id": track_id,
        "userHash": "{\"level\":\"LIBRARY_MEMBER\"}",
        "headers": headers_obj.to_string()
    });

    let res = tokio::time::timeout(std::time::Duration::from_secs(4), client.post(CATALOG_TRACK_URL)
        .header("User-Agent", SEARCH_USER_AGENT)
        .header("Content-Type", "text/plain;charset=UTF-8")
        .header("Origin", "https://music.amazon.com")
        .header("Referer", "https://music.amazon.com/")
        .body(payload.to_string())
        .send()).await.ok()?.ok()?;

    if !res.status().is_success() {
        return None;
    }

    let data: Value = res.json().await.ok()?;
    let t_data = &data["methods"][0]["template"];
    
    // 1. Initial metadata from header
    let mut title = get_text(&t_data["headerPrimaryText"], "Unknown Track");
    let mut author = get_text(&t_data["headerSecondaryText"], "Unknown Artist");
    
    // 2. Exhaustive metadata scan from items and widgets
    let mut found_title = String::new();
    let mut found_author = String::new();

    let check_meta = |obj: &Value| -> (String, String) {
        let t = get_text(&obj["primaryText"], "");
        let a0 = get_text(&obj["secondaryText"], "");
        let a1 = get_text(&obj["secondaryText1"], "");
        let a2 = get_text(&obj["secondaryText2"], "");
        let a3 = get_text(&obj["secondaryText3"], "");
        
        let mut a = String::new();
        for val in [a0, a1, a2, a3] {
            if !val.is_empty() && val != "Unknown Artist" && !is_duration_like(&val) {
                a = val;
                break;
            }
        }
        (t, a)
    };

    // Helper for artwork
    let check_art = |obj: &Value| -> Option<String> {
        if let Some(s) = obj.as_str() { return Some(s.to_string()); }
        if let Some(s) = obj["src"].as_str() { return Some(s.to_string()); }
        if let Some(s) = obj["image"].as_str() { return Some(s.to_string()); }
        if let Some(s) = obj["image"]["src"].as_str() { return Some(s.to_string()); }
        if let Some(s) = obj["headerImage"]["src"].as_str() { return Some(s.to_string()); }
        None
    };

    let mut artwork_url = check_art(&t_data["headerImage"]);
    if artwork_url.is_none() { artwork_url = check_art(&t_data["headerLink"]["image"]); }

    if let Some(items) = t_data["items"].as_array() {
        for item in items {
            let (t, a) = check_meta(item);
            if !t.is_empty() && found_title.is_empty() { found_title = t; }
            if !a.is_empty() && found_author.is_empty() { found_author = a; }
            if artwork_url.is_none() { artwork_url = check_art(item); }
        }
    }

    if let Some(widgets) = t_data["widgets"].as_array() {
        for widget in widgets {
            if let Some(items) = widget["items"].as_array() {
                for item in items {
                    let (t, a) = check_meta(item);
                    if !t.is_empty() && found_title.is_empty() { found_title = t; }
                    if !a.is_empty() && found_author.is_empty() { found_author = a; }
                    if artwork_url.is_none() { artwork_url = check_art(item); }
                }
            }
        }
    }

    if !found_title.is_empty() { title = found_title; }
    if !found_author.is_empty() { author = found_author; }

    let length_str = t_data["headerTertiaryText"].as_str().unwrap_or("");
    let length = parse_time_string_to_ms(length_str);

    Some(LoadResult::Track(Track::new(TrackInfo {
        identifier: track_id.to_string(),
        is_seekable: true,
        author,
        length,
        is_stream: false,
        position: 0,
        title,
        uri: Some(format!("https://music.amazon.com/tracks/{}", track_id)),
        artwork_url,
        isrc: None,
        source_name: "amazonmusic".to_string(),
    })))
}
