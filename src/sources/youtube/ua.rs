pub mod yt_ua {
  pub const IOS: &str =
    "com.google.ios.youtube/21.02.1 (iPhone16,2; U; CPU iOS 18_2 like Mac OS X;)";
  pub const ANDROID: &str = "com.google.android.youtube/20.01.35 (Linux; U; Android 14) identity";
  pub const ANDROID_VR: &str = "Mozilla/5.0 (Linux; Android 14; Pixel 8 Pro Build/UQ1A.240205.002; wv) \
         AppleWebKit/537.36 (KHTML, like Gecko) Version/4.0 \
         Chrome/121.0.6167.164 Mobile Safari/537.36 YouTubeVR/1.42.15 (gzip)";
  pub const TVHTML5: &str = "Mozilla/5.0 (Fuchsia) AppleWebKit/537.36 (KHTML, like Gecko) \
         Chrome/140.0.0.0 Safari/537.36 CrKey/1.56.500000";
  pub const MWEB: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_2 like Mac OS X) \
         AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1";
  pub const WEB_EMBEDDED: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
         (KHTML, like Gecko) Chrome/134.0.0.0 Safari/537.36";
}

pub fn get_youtube_ua(url: &str) -> Option<&'static str> {
  if !(url.contains("googlevideo.com") || url.contains("youtube.com")) {
    return None;
  }

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

fn extract_param<'a>(url: &'a str, key: &str) -> Option<&'a str> {
  let query_start = url.find('?')?;
  let query = &url[query_start + 1..];

  for part in query.split('&') {
    if let Some(val) = part.strip_prefix(key) {
      return Some(val.split('#').next().unwrap_or(val));
    }
  }
  None
}
