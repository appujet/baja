use std::sync::{Arc, OnceLock};

use regex::Regex;
use serde_json::{Value, json};

use super::YouTubeCipherManager;
use crate::common::types::AnyResult;

pub const INNERTUBE_API: &str = "https://youtubei.googleapis.com";

#[derive(Debug, Clone)]
pub struct ClientConfig<'a> {
    pub client_name: &'a str,
    pub client_version: &'a str,
    pub client_id: &'a str,
    pub user_agent: &'a str,
    pub os_name: Option<&'a str>,
    pub os_version: Option<&'a str>,
    pub device_make: Option<&'a str>,
    pub device_model: Option<&'a str>,
    pub platform: Option<&'a str>,
    pub android_sdk_version: Option<&'a str>,
    pub hl: &'a str,
    pub gl: &'a str,
    pub utc_offset_minutes: Option<i32>,
    pub third_party_embed_url: Option<&'a str>,
}

impl<'a> Default for ClientConfig<'a> {
    fn default() -> Self {
        Self {
            client_name: "",
            client_version: "",
            client_id: "",
            user_agent: "",
            os_name: None,
            os_version: None,
            device_make: None,
            device_model: None,
            platform: None,
            android_sdk_version: None,
            hl: "en",
            gl: "US",
            utc_offset_minutes: None,
            third_party_embed_url: None,
        }
    }
}

impl<'a> ClientConfig<'a> {
    pub fn build_context(&self, visitor_data: Option<&str>) -> Value {
        let mut client = json!({
            "clientName": self.client_name,
            "clientVersion": self.client_version,
            "userAgent": self.user_agent,
            "hl": self.hl,
            "gl": self.gl,
        });

        if let Some(obj) = client.as_object_mut() {
            if let Some(v) = self.os_name {
                obj.insert("osName".to_string(), v.into());
            }
            if let Some(v) = self.os_version {
                obj.insert("osVersion".to_string(), v.into());
            }
            if let Some(v) = self.device_make {
                obj.insert("deviceMake".to_string(), v.into());
            }
            if let Some(v) = self.device_model {
                obj.insert("deviceModel".to_string(), v.into());
            }
            if let Some(v) = self.platform {
                obj.insert("platform".to_string(), v.into());
            }
            if let Some(v) = self.android_sdk_version {
                obj.insert("androidSdkVersion".to_string(), v.into());
            }
            if let Some(v) = self.utc_offset_minutes {
                obj.insert("utcOffsetMinutes".to_string(), v.into());
            }
            if let Some(vd) = visitor_data {
                obj.insert("visitorData".to_string(), vd.into());
            }
        }

        let mut context = json!({
            "client": client,
            "user": { "lockedSafetyMode": false },
            "request": { "useSsl": true }
        });

        if let Some(url) = self.third_party_embed_url {
            if let Some(obj) = context.as_object_mut() {
                obj.insert("thirdParty".to_string(), json!({ "embedUrl": url }));
            }
        }

        context
    }
}

pub const AUDIO_ITAG_PRIORITY: &[i64] = &[251, 250, 140]; // 251/250 are opus, 140 is aac

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
) -> AnyResult<Option<String>> {
    // Plain URL path
    if let Some(url) = format.get("url").and_then(|u| u.as_str()) {
        // n-param throttling: must be decoded via cipher
        let n_param = url
            .split("&n=")
            .nth(1)
            .or_else(|| url.split("?n=").nth(1))
            .and_then(|s| s.split('&').next());

        // If there's no n-param to decode, return the URL directly — no cipher call needed.
        // (e.g. AndroidVR, TV responses often omit the n throttle param entirely)
        if n_param.is_none() {
            return Ok(Some(url.to_string()));
        }

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
    config: &ClientConfig<'_>,
    video_id: &str,
    params: Option<&str>,
    visitor_data: Option<&str>,
    signature_timestamp: Option<u32>,
    auth_header: Option<String>,
    referer: Option<&str>,
    origin: Option<&str>,
    po_token: Option<&str>,
) -> AnyResult<Value> {
    let mut body = json!({
        "context": config.build_context(visitor_data),
        "videoId": video_id,
        "contentCheckOk": true,
        "racyCheckOk": true
    });

    if let Some(token) = po_token {
        if let Some(obj) = body.as_object_mut() {
            obj.insert(
                "serviceIntegrityDimensions".to_string(),
                json!({ "poToken": token }),
            );
        }
    }

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
        .header("User-Agent", config.user_agent)
        .header("X-YouTube-Client-Name", config.client_id)
        .header("X-YouTube-Client-Version", config.client_version)
        .header("X-Goog-Api-Format-Version", "2");

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

pub async fn make_next_request(
    http: &reqwest::Client,
    config: &ClientConfig<'_>,
    video_id: Option<&str>,
    playlist_id: Option<&str>,
    visitor_data: Option<&str>,
    auth_header: Option<String>,
) -> AnyResult<Value> {
    let mut body = json!({
        "context": config.build_context(visitor_data),
    });

    if let Some(vid) = video_id {
        if let Some(obj) = body.as_object_mut() {
            obj.insert("videoId".to_string(), vid.into());
        }
    }

    if let Some(pid) = playlist_id {
        if let Some(obj) = body.as_object_mut() {
            obj.insert("playlistId".to_string(), pid.into());
        }
    }

    let url = format!("{}/youtubei/v1/next?prettyPrint=false", INNERTUBE_API);

    let mut req = http
        .post(&url)
        .header("User-Agent", config.user_agent)
        .header("X-YouTube-Client-Name", config.client_id)
        .header("X-YouTube-Client-Version", config.client_version);

    if let Some(vd) = visitor_data {
        req = req.header("X-Goog-Visitor-Id", vd);
    }

    if let Some(auth) = auth_header {
        req = req.header("Authorization", auth);
    }

    let res = req.json(&body).send().await?;
    let status = res.status();
    if !status.is_success() {
        let text = res
            .text()
            .await
            .unwrap_or_else(|_| "Unknown error".to_string());
        return Err(format!("Next request failed (status={}): {}", status, text).into());
    }

    Ok(res.json().await?)
}
