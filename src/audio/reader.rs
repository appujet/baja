use std::io::{Read, Seek, SeekFrom};
use symphonia::core::io::MediaSource;

/// UA strings must exactly match what the respective InnerTube clients use when
/// fetching the player response, otherwise YouTube's CDN returns 403.
///
/// These are kept in sync with the constants defined in each client module.
mod yt_ua {
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

/// Detect the YouTube InnerTube client name embedded in a `googlevideo.com`
/// playback URL and return the matching `User-Agent` string.
///
/// YouTube's CDN validates that the UA on the audio fetch matches the UA that
/// was used when the `/player` API was called.  A mismatch (including a version
/// mismatch) produces a `403 Forbidden`.
fn youtube_user_agent_for_url(url: &str) -> Option<&'static str> {
    // `c=<CLIENT>` appears in the query string of googlevideo CDN URLs.
    if !(url.contains("googlevideo.com") || url.contains("youtube.com")) {
        return None;
    }

    // Parse `c=` parameter robustly — it could appear anywhere in the QS.
    let client_param = url
        .split('&')
        .chain(url.split('?').nth(1).into_iter().flat_map(|q| q.split('&')))
        .find_map(|kv| kv.strip_prefix("c="))?;

    match client_param {
        "IOS" => Some(yt_ua::IOS),
        "ANDROID" => Some(yt_ua::ANDROID),
        "ANDROID_VR" => Some(yt_ua::ANDROID_VR),
        "TVHTML5" => Some(yt_ua::TVHTML5),
        "MWEB" => Some(yt_ua::MWEB),
        "WEB_EMBEDDED_PLAYER" => Some(yt_ua::WEB_EMBEDDED),
        _ => None,
    }
}

// ─── RemoteReader ─────────────────────────────────────────────────────────────

pub struct RemoteReader {
    url: String,
    client: reqwest::blocking::Client,
    response: reqwest::blocking::Response,
    pos: u64,
    len: Option<u64>,
}

impl RemoteReader {
    pub fn new(
        url: &str,
        local_addr: Option<std::net::IpAddr>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        // Prefer the UA that matches the YouTube InnerTube client; fall back to
        // a generic browser UA for non-YouTube sources.
        let user_agent = youtube_user_agent_for_url(url)
            .map(str::to_string)
            .unwrap_or_else(crate::common::http::HttpClient::random_user_agent);

        let mut builder = reqwest::blocking::Client::builder()
            .user_agent(user_agent)
            .timeout(std::time::Duration::from_secs(15))
            .connection_verbose(false);

        if let Some(ip) = local_addr {
            builder = builder.local_address(ip);
        }

        let client = builder.build()?;

        let response = client
            .get(url)
            .header("Accept", "*/*")
            .header("Accept-Encoding", "identity") // avoid gzip so Read works correctly
            .send()?;

        if !response.status().is_success() {
            return Err(format!("Server returned status {}: {}", response.status(), url).into());
        }

        let len = response.content_length();

        Ok(Self {
            url: url.to_string(),
            client,
            response,
            pos: 0,
            len,
        })
    }

    pub fn content_type(&self) -> Option<String> {
        self.response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    }
}

// ─── I/O traits ───────────────────────────────────────────────────────────────

impl Read for RemoteReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.response.read(buf)?;
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for RemoteReader {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        let new_pos = match pos {
            SeekFrom::Start(p) => p,
            SeekFrom::Current(delta) => self.pos.saturating_add_signed(delta),
            SeekFrom::End(delta) => {
                let len = self.len.ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::Unsupported,
                        "Cannot seek from end: content length unknown",
                    )
                })?;
                len.saturating_add_signed(delta)
            }
        };

        if new_pos == self.pos {
            return Ok(self.pos);
        }

        let range = format!("bytes={}-", new_pos);
        self.response = self
            .client
            .get(&self.url)
            .header("Range", range)
            .header("Accept", "*/*")
            .header("Accept-Encoding", "identity")
            .send()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

        self.pos = new_pos;
        Ok(self.pos)
    }
}

impl MediaSource for RemoteReader {
    fn is_seekable(&self) -> bool {
        self.len.is_some()
    }

    fn byte_len(&self) -> Option<u64> {
        self.len
    }
}
