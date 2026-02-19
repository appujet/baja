use crate::sources::youtube::cipher::YouTubeCipherManager;
use serde_json::Value;
use std::sync::Arc;

/// YouTube InnerTube API base endpoint (googleapis is more stable and avoids
/// some geo-restrictions that www.youtube.com may impose).
pub const INNERTUBE_API: &str = "https://youtubei.googleapis.com";

/// Audio itag priority order, matching NodeLink's `_getQualityPriority` "high".
/// 251 = Opus/WebM ~160 kbps, 250 = Opus/WebM ~70 kbps, 140 = AAC/m4a 128 kbps
pub const AUDIO_ITAG_PRIORITY: &[i64] = &[251, 250, 140];

/// Fallback itag (360p mp4 with audio - always available on most videos).
pub const ITAG_FALLBACK: i64 = 18;

/// Decode a `signatureCipher` / `cipher` query string into (url, sig) parts.
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

/// Priority-based audio format selector.
///
/// Returns the best audio format according to itag priority list.
/// Falls back to any audio format by highest bitrate, then to itag 18.
pub fn select_best_audio_format<'a>(
    adaptive_formats: Option<&'a Vec<Value>>,
    formats: Option<&'a Vec<Value>>,
) -> Option<&'a Value> {
    let all: Vec<&Value> = adaptive_formats
        .into_iter()
        .flatten()
        .chain(formats.into_iter().flatten())
        .collect();

    // Pass 1: try priority itags in order
    for &target_itag in AUDIO_ITAG_PRIORITY {
        for f in &all {
            let itag = f.get("itag").and_then(|v| v.as_i64()).unwrap_or(-1);
            let mime = f.get("mimeType").and_then(|v| v.as_str()).unwrap_or("");
            if itag == target_itag && mime.starts_with("audio/") {
                return Some(f);
            }
        }
    }

    // Pass 2: fallback to itag 18 (360p muxed - always has audio)
    for f in &all {
        let itag = f.get("itag").and_then(|v| v.as_i64()).unwrap_or(-1);
        if itag == ITAG_FALLBACK {
            return Some(f);
        }
    }

    // Pass 3: any audio format by highest bitrate
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

/// Resolve a format's stream URL, handling both plain `url` fields and
/// `signatureCipher` / `cipher` encoded fields.
///
/// Returns `Ok(Some(url))` on success, `Ok(None)` when no resolvable URL
/// exists in the format, and `Err` on cipher resolution failure.
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
    // Note: n-parameter can also be present in formats that DON'T have a plain url yet,
    // but the regex/split above handles it once the url is extracted.

    // signatureCipher / cipher path
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
