use serde_json::Value;
use regex::Regex;
use super::token::AmazonConfig;

pub const SEARCH_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/144.0.0.0 Safari/537.36";
pub const CATALOG_TRACK_URL: &str = "https://na.mesk.skill.music.a2z.com/api/cosmicTrack/displayCatalogTrack";

pub fn extract_track_asin_param(u: &str) -> Option<String> {
    let k = "trackAsin=";
    let i = u.find(k)?;
    let s = i + k.len();
    
    let mut e = u[s..].find('&').map(|pos| s + pos).unwrap_or(u.len());
    let e2 = u[s..].find("%26").map(|pos| s + pos).unwrap_or(u.len());
    if e2 < e { e = e2; }
    let h = u[s..].find('#').map(|pos| s + pos).unwrap_or(u.len());
    if h < e { e = h; }

    let id = &u[s..e];
    if id.is_empty() { None } else { Some(id.to_string()) }
}

pub fn extract_identifier(deeplink: &str) -> Option<String> {
    if deeplink.is_empty() { return None; }
    if let Some(asin) = extract_track_asin_param(deeplink) {
        return Some(asin);
    }

    let mut end = deeplink.len();
    if let Some(q) = deeplink.find('?') {
        if q < end { end = q; }
    }
    if let Some(h) = deeplink.find('#') {
        if h < end { end = h; }
    }

    let cut = deeplink[..end].rfind('/')?;
    let id = &deeplink[cut + 1..end];
    if id.is_empty() { None } else { Some(id.to_string()) }
}

pub fn parse_iso8601_duration(duration: &str) -> u64 {
    let re = Regex::new(r"PT(?:(\d+)H)?(?:(\d+)M)?(?:(\d+)S)?").unwrap();
    let caps = match re.captures(duration) {
        Some(c) => c,
        None => return 0,
    };

    let hours: u64 = caps.get(1).map_or(0, |m| m.as_str().parse().unwrap_or(0));
    let minutes: u64 = caps.get(2).map_or(0, |m| m.as_str().parse().unwrap_or(0));
    let seconds: u64 = caps.get(3).map_or(0, |m| m.as_str().parse().unwrap_or(0));

    (hours * 3600 + minutes * 60 + seconds) * 1000
}

pub fn parse_time_string_to_ms(s: &str) -> u64 {
    let s = s.to_uppercase();
    let mut total = 0;
    
    let mut i = 0;
    let bytes = s.as_bytes();
    while i < bytes.len() {
        let c = bytes[i];
        if !c.is_ascii_digit() {
            i += 1;
            continue;
        }

        let mut n = 0;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            n = n * 10 + (bytes[i] - b'0') as u64;
            i += 1;
        }

        while i < bytes.len() && bytes[i] == b' ' { i += 1; }

        if s[i..].starts_with("HOUR") {
            total += n * 3600;
        } else if s[i..].starts_with("MINUTE") {
            total += n * 60;
        } else if s[i..].starts_with("SECOND") {
            total += n;
        }
        i += 1;
    }
    total * 1000
}

pub fn get_text(v: &Value, fallback: &str) -> String {
    if v.is_null() {
        return fallback.to_string();
    }
    // Handle nested text: { text: { text: "..." } } or { text: "..." }
    if let Some(text_obj) = v.get("text") {
        if let Some(s) = text_obj.as_str() {
            return decode_amp(s);
        }
        return get_text(text_obj, fallback);
    }
    if let Some(s) = v.as_str() {
        return decode_amp(s);
    }
    fallback.to_string()
}

pub fn is_duration_like(s: &str) -> bool {
    let s_clean = s.trim().to_uppercase();
    if s_clean.is_empty() { return false; }
    
    // Check for HH:MM:SS or MM:SS
    if s_clean.contains(':') {
        let parts: Vec<&str> = s_clean.split(':').collect();
        if parts.len() >= 2 && parts.iter().all(|p| p.chars().all(|c| c.is_ascii_digit())) {
            return true;
        }
    }
    
    // Check for "3 MINUTES", "20 SECONDS" etc
    if s_clean.contains("MINUTE") || s_clean.contains("SECOND") || s_clean.contains("HOUR") {
        return true;
    }
    
    false
}

pub fn parse_colon_duration_to_ms(s: &str) -> u64 {
    let parts: Vec<&str> = s.split(':').collect();
    let mut sec = 0;
    for part in parts {
        if let Ok(n) = part.parse::<u64>() {
            sec = sec * 60 + n;
        } else {
            return 0;
        }
    }
    sec * 1000
}

pub fn decode_amp(s: &str) -> String {
    let mut decoded = s.to_string();
    
    if !decoded.contains('&') {
        return decoded;
    }

    let named_entities = [
        ("&amp;", "&"),
        ("&quot;", "\""),
        ("&lt;", "<"),
        ("&gt;", ">"),
        ("&#39;", "'"),
        ("&apos;", "'"),
        ("&nbsp;", " "),
        ("&copy;", "©"),
        ("&reg;", "®"),
        ("&euro;", "€"),
        ("&pound;", "£"),
        ("&yen;", "¥"),
        ("&cent;", "¢"),
        ("&ndash;", "–"),
        ("&mdash;", "—"),
        ("&lsquo;", "‘"),
        ("&rsquo;", "’"),
        ("&sbquo;", "‚"),
        ("&ldquo;", "“"),
        ("&rdquo;", "”"),
        ("&bdquo;", "„"),
        ("&dagger;", "†"),
        ("&Dagger;", "‡"),
        ("&bull;", "•"),
        ("&hellip;", "…"),
        ("&prime;", "′"),
        ("&Prime;", "″"),
        ("&tilde;", "˜"),
        ("&trade;", "™")
    ];

    for (entity, replacement) in named_entities.iter() {
        if decoded.contains(entity) {
            decoded = decoded.replace(entity, replacement);
        }
    }

    if decoded.contains("&#") {
        let re_dec = Regex::new(r"&#(\d+);").unwrap();
        decoded = re_dec.replace_all(&decoded, |caps: &regex::Captures| {
            if let Ok(num) = caps[1].parse::<u32>() {
                if let Some(c) = char::from_u32(num) {
                    return c.to_string();
                }
            }
            caps[0].to_string()
        }).to_string();

        let re_hex = Regex::new(r"&#[xX]([0-9a-fA-F]+);").unwrap();
        decoded = re_hex.replace_all(&decoded, |caps: &regex::Captures| {
            if let Ok(num) = u32::from_str_radix(&caps[1], 16) {
                if let Some(c) = char::from_u32(num) {
                    return c.to_string();
                }
            }
            caps[0].to_string()
        }).to_string();
    }

    decoded = decoded.replace("&amp;", "&");
    decoded
}

pub fn build_amazon_headers(
    cfg: &AmazonConfig,
    now: u128,
    csrf_header: &str,
    page_url: &str,
) -> Value {
    serde_json::json!({
        "x-amzn-authentication": serde_json::json!({
            "interface": "ClientAuthenticationInterface.v1_0.ClientTokenElement",
            "accessToken": cfg.access_token
        }).to_string(),
        "x-amzn-device-model": "WEBPLAYER",
        "x-amzn-device-width": "1920",
        "x-amzn-device-height": "1080",
        "x-amzn-device-family": "WebPlayer",
        "x-amzn-device-id": cfg.device_id,
        "x-amzn-user-agent": SEARCH_USER_AGENT,
        "x-amzn-session-id": cfg.session_id,
        "x-amzn-request-id": uuid::Uuid::new_v4().to_string(),
        "x-amzn-device-language": "en_US",
        "x-amzn-currency-of-preference": "USD",
        "x-amzn-os-version": "1.0",
        "x-amzn-application-version": "1.0.9172.0",
        "x-amzn-device-time-zone": "America/New_York",
        "x-amzn-timestamp": now.to_string(),
        "x-amzn-csrf": csrf_header,
        "x-amzn-music-domain": "music.amazon.com",
        "x-amzn-page-url": page_url,
        "x-amzn-feature-flags": "hd-supported,uhd-supported",
        "x-amzn-referer": "",
        "x-amzn-affiliate-tags": "",
        "x-amzn-ref-marker": "",
        "x-amzn-weblab-id-overrides": "",
        "x-amzn-video-player-token": "",
        "x-amzn-has-profile-id": "",
        "x-amzn-age-band": ""
    })
}
