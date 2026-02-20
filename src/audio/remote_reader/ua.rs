pub mod yt_ua {
    pub const IOS: &str =
        "com.google.ios.youtube/21.02.1 (iPhone16,2; U; CPU iOS 18_2 like Mac OS X;)";
    pub const ANDROID: &str = "com.google.android.youtube/20.01.35 (Linux; U; Android 14) identity";
    pub const ANDROID_VR: &str = "Mozilla/5.0 (Linux; Android 14; Pixel 8 Pro Build/UQ1A.240205.002; wv) \
         AppleWebKit/537.36 (KHTML, like Gecko) Version/4.0 \
         Chrome/121.0.6167.164 Mobile Safari/537.36 YouTubeVR/1.42.15 (gzip)";
    pub const TVHTML5: &str = "Mozilla/5.0 (SmartHub; SMART-TV; U; Linux/SmartTV; Maple2012) \
         AppleWebKit/534.7 (KHTML, like Gecko) SmartTV Safari/534.7";
    pub const MWEB: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_2 like Mac OS X) \
         AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1";
    pub const WEB_EMBEDDED: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
         (KHTML, like Gecko) Chrome/134.0.0.0 Safari/537.36";
}

/// Detects and returns the appropriate YouTube User-Agent for a given URL.
///
/// It looks for the 'c=' parameter in the query string, which identifies the
/// client that generated the playback URL.
pub fn get_youtube_ua(url: &str) -> Option<&'static str> {
    // Only process YouTube-related domains to avoid unnecessary string ops
    if !(url.contains("googlevideo.com") || url.contains("youtube.com")) {
        return None;
    }

    // Pro-tip: Manual param extraction is faster than full URL parsing in high-frequency loops.
    extract_param(url, "c=").and_then(|client| match client {
        "IOS" => Some(yt_ua::IOS),
        "ANDROID" => Some(yt_ua::ANDROID),
        "ANDROID_VR" => Some(yt_ua::ANDROID_VR),
        "TVHTML5" => Some(yt_ua::TVHTML5),
        "MWEB" => Some(yt_ua::MWEB),
        "WEB_EMBEDDED_PLAYER" => Some(yt_ua::WEB_EMBEDDED),
        _ => None,
    })
}

/// Robustly extracts a query parameter value. Handles both '?' and '&' boundaries.
fn extract_param<'a>(url: &'a str, key: &str) -> Option<&'a str> {
    let query_start = url.find('?')?;
    let query = &url[query_start + 1..];

    for part in query.split('&') {
        if let Some(val) = part.strip_prefix(key) {
            // Trim potential fragment identifiers at the end of the last param
            return Some(val.split('#').next().unwrap_or(val));
        }
    }
    None
}
