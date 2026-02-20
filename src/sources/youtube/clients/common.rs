use super::YouTubeCipherManager;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use regex::Regex;
use serde_json::{Value, json};
use std::sync::{Arc, OnceLock};

pub const INNERTUBE_API: &str = "https://youtubei.googleapis.com";

pub const AUDIO_ITAG_PRIORITY: &[i64] = &[251, 250, 140];

pub const ITAG_FALLBACK: i64 = 18;

pub fn decode_signature_cipher(cipher_str: &str) -> Option<(String, String)> {
    let mut url = None;
    let mut sig = None;

    for part in cipher_str.split('&') {
        if let Some((k, v)) = part.split_once('=') {
            let decoded = urlencoding::decode(v).ok()?.to_string();
            match k {
                "url" => url = Some(decoded),
                "s" => sig = Some(decoded),
                _ => {}
            }
        }
    }

    match (url, sig) {
        (Some(u), Some(s)) => Some((u, s)),
        _ => None,
    }
}

pub fn select_best_audio_format<'a>(
    adaptive_formats: Option<&'a Vec<Value>>,
    formats: Option<&'a Vec<Value>>,
) -> Option<&'a Value> {
    let all: Vec<&Value> = adaptive_formats
        .into_iter()
        .flatten()
        .chain(formats.into_iter().flatten())
        .collect();

    for &target_itag in AUDIO_ITAG_PRIORITY {
        for f in &all {
            let itag = f.get("itag").and_then(|v| v.as_i64()).unwrap_or(-1);
            let mime = f.get("mimeType").and_then(|v| v.as_str()).unwrap_or("");
            if itag == target_itag && mime.starts_with("audio/") {
                return Some(f);
            }
        }
    }

    for f in &all {
        let itag = f.get("itag").and_then(|v| v.as_i64()).unwrap_or(-1);
        if itag == ITAG_FALLBACK {
            return Some(f);
        }
    }

    let mut best: Option<&Value> = None;
    let mut best_bitrate = 0i64;
    for f in all {
        let mime = f.get("mimeType").and_then(|v| v.as_str()).unwrap_or("");
        if mime.starts_with("audio/") {
            let bitrate = f.get("bitrate").and_then(|v| v.as_i64()).unwrap_or(0);
            if bitrate > best_bitrate {
                best = Some(f);
                best_bitrate = bitrate;
            }
        }
    }
    best
}

pub async fn resolve_format_url(
    format: &Value,
    player_page_url: &str,
    cipher_manager: &Arc<YouTubeCipherManager>,
) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
    // Plain URL path
    if let Some(url) = format.get("url").and_then(|u| u.as_str()) {
        // n-param throttling: must be decoded via cipher
        let n_param = url
            .split("&n=")
            .nth(1)
            .or_else(|| url.split("?n=").nth(1))
            .and_then(|s| s.split('&').next());
        let resolved = cipher_manager
            .resolve_url(url, player_page_url, n_param, None)
            .await?;
        return Ok(Some(resolved));
    }

    let cipher_str = format
        .get("signatureCipher")
        .or_else(|| format.get("cipher"))
        .and_then(|c| c.as_str());

    if let Some(cipher_str) = cipher_str {
        if let Some((url, sig)) = decode_signature_cipher(cipher_str) {
            let n_param = url
                .split("&n=")
                .nth(1)
                .or_else(|| url.split("?n=").nth(1))
                .and_then(|s| s.split('&').next());
            let resolved = cipher_manager
                .resolve_url(&url, player_page_url, n_param, Some(&sig))
                .await?;
            return Ok(Some(resolved));
        }
    }

    Ok(None)
}

static DURATION_REGEX: OnceLock<Regex> = OnceLock::new();

pub fn is_duration(text: &str) -> bool {
    let re = DURATION_REGEX.get_or_init(|| Regex::new(r"^\d{1,2}:\d{2}(:\d{2})?$").unwrap());
    re.is_match(text)
}

pub fn parse_duration(duration: &str) -> u64 {
    let parts: Vec<&str> = duration.split(':').collect();
    let mut ms = 0u64;
    for part in parts {
        if let Ok(num) = part.parse::<u64>() {
            ms = ms * 60 + num;
        }
    }
    ms * 1000
}

pub fn extract_thumbnail(renderer: &Value, video_id: Option<&str>) -> Option<String> {
    let thumbnails = renderer
        .get("thumbnail")
        .and_then(|t| t.get("thumbnails"))
        .or_else(|| {
            renderer
                .get("thumbnail")
                .and_then(|t| t.get("musicThumbnailRenderer"))
                .and_then(|t| t.get("thumbnail"))
                .and_then(|t| t.get("thumbnails"))
        });

    if let Some(thumbnails) = thumbnails.and_then(|t| t.as_array()) {
        if !thumbnails.is_empty() {
            if let Some(url) = thumbnails
                .last()
                .and_then(|t| t.get("url"))
                .and_then(|u| u.as_str())
            {
                return Some(url.split('?').next().unwrap_or(url).to_string());
            }
        }
    }

    if let Some(id) = video_id {
        return Some(format!("https://i.ytimg.com/vi/{}/hqdefault.jpg", id));
    }

    None
}

pub async fn make_player_request(
    http: &reqwest::Client,
    video_id: &str,
    context: Value,
    client_id: &str,
    client_version: &str,
    params: Option<&str>,
    visitor_data: Option<&str>,
    signature_timestamp: Option<u32>,
    auth_header: Option<String>,
    referer: Option<&str>,
    origin: Option<&str>,
) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
    let mut body = json!({
        "context": context,
        "videoId": video_id,
        "contentCheckOk": true,
        "racyCheckOk": true
    });

    if let Some(p) = params {
        if let Some(obj) = body.as_object_mut() {
            obj.insert("params".to_string(), p.into());
        }
    }

    if let Some(sts) = signature_timestamp {
        if let Some(obj) = body.as_object_mut() {
            obj.insert(
                "playbackContext".to_string(),
                json!({
                    "contentPlaybackContext": {
                        "signatureTimestamp": sts
                    }
                }),
            );
        }
    }

    let url = format!("{}/youtubei/v1/player?prettyPrint=false", INNERTUBE_API);

    let mut req = http
        .post(&url)
        .header("X-YouTube-Client-Name", client_id)
        .header("X-YouTube-Client-Version", client_version);

    if let Some(vd) = visitor_data {
        req = req.header("X-Goog-Visitor-Id", vd);
    }

    if let Some(auth) = auth_header {
        req = req.header("Authorization", auth);
    }

    if let Some(ref_url) = referer {
        req = req.header("Referer", ref_url);
    }

    if let Some(orig_url) = origin {
        req = req.header("Origin", orig_url);
    }

    let res = req.json(&body).send().await?;
    let status = res.status();
    if !status.is_success() {
        let text = res
            .text()
            .await
            .unwrap_or_else(|_| "Unknown error".to_string());
        return Err(format!("Player request failed (status={}): {}", status, text).into());
    }

    Ok(res.json().await?)
}

pub fn extract_sabr_payload(
    body: &Value,
    streaming_data: &Value,
    client_id: i64,
    client_version: &str,
) -> Option<String> {
    let server_abr_url = streaming_data.get("serverAbrStreamingUrl")?.as_str()?;
    let ustreamer_config = body
        .get("playerConfig")
        .and_then(|p| p.get("mediaCommonConfig"))
        .and_then(|m| m.get("mediaUstreamerRequestConfig"))
        .and_then(|m| m.get("videoPlaybackUstreamerConfig"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let visitor_data = body
        .get("responseContext")
        .and_then(|r| r.get("visitorData"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let formats: Vec<Value> = streaming_data
        .get("adaptiveFormats")
        .and_then(|f| f.as_array())
        .into_iter()
        .flatten()
        .map(|f| {
            json!({
                "itag": f.get("itag").and_then(|v| v.as_i64()).unwrap_or(0),
                "lastModified": f.get("lastModified")
                    .or_else(|| f.get("last_modified_ms"))
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse::<i64>().ok()),
                "xtags": f.get("xtags").and_then(|v| v.as_str()),
                "mimeType": f.get("mimeType").and_then(|v| v.as_str()),
                "bitrate": f.get("bitrate").and_then(|v| v.as_i64()),
                "audioQuality": f.get("audioQuality").and_then(|v| v.as_str()),
            })
        })
        .collect();

    let sabr_payload = json!({
        "url":           server_abr_url,
        "config":        ustreamer_config,
        "clientName":    client_id,
        "clientVersion": client_version,
        "visitorData":   visitor_data,
        "formats":       formats,
    });

    let encoded = BASE64_STANDARD.encode(serde_json::to_string(&sabr_payload).ok()?);
    Some(format!("sabr://{}", encoded))
}