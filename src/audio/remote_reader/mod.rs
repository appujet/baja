pub mod ua;

use std::io::{Read, Seek, SeekFrom};
use symphonia::core::io::MediaSource;
use tracing::{debug, info};

/// A professional-grade HTTP stream reader with YouTube-specific optimizations
/// and robust seek support via Range headers.
pub struct RemoteReader {
    /// The canonical URL being read from.
    url: String,
    /// Persistent blocking HTTP client for the session.
    client: reqwest::blocking::Client,
    /// The current active HTTP response body stream.
    response: reqwest::blocking::Response,
    /// The absolute byte position within the stream.
    pos: u64,
    /// Total content length if reported by the remote server.
    len: Option<u64>,
}

impl RemoteReader {
    /// Creates a new RemoteReader and initiates the first GET request.
    ///
    /// Automatically selects a suitable User-Agent based on the URL (optimized for YouTube).
    pub fn new(
        url: &str,
        local_addr: Option<std::net::IpAddr>,
        proxy: Option<crate::configs::HttpProxyConfig>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let user_agent = ua::get_youtube_ua(url)
            .map(str::to_string)
            .unwrap_or_else(crate::common::http::HttpClient::random_user_agent);

        let mut builder = reqwest::blocking::Client::builder()
            .user_agent(user_agent)
            .timeout(std::time::Duration::from_secs(15));

        if let Some(ip) = local_addr {
            builder = builder.local_address(ip);
        }

        if let Some(proxy_config) = proxy {
            if let Some(p_url) = &proxy_config.url {
                if let Ok(mut proxy_obj) = reqwest::Proxy::all(p_url) {
                    if let (Some(u), Some(p)) = (proxy_config.username, proxy_config.password) {
                        proxy_obj = proxy_obj.basic_auth(&u, &p);
                    }
                    builder = builder.proxy(proxy_obj);
                    debug!("Configured proxy for RemoteReader: {}", p_url);
                }
            }
        }

        let client = builder.build()?;
        let response = Self::fetch_stream(&client, url, 0)?;
        let len = response.content_length();

        info!("Opened RemoteReader: {} (len={:?})", url, len);

        Ok(Self {
            url: url.to_string(),
            client,
            response,
            pos: 0,
            len,
        })
    }

    /// Internal helper to perform a Range request.
    fn fetch_stream(
        client: &reqwest::blocking::Client,
        url: &str,
        offset: u64,
    ) -> Result<reqwest::blocking::Response, Box<dyn std::error::Error + Send + Sync>> {
        let mut req = client
            .get(url)
            .header("Accept", "*/*")
            .header("Accept-Encoding", "identity");

        if offset > 0 {
            req = req.header("Range", format!("bytes={}-", offset));
        }

        let res = req.send()?;
        if !res.status().is_success() {
            return Err(format!("Stream fetch failed ({}): {}", res.status(), url).into());
        }
        Ok(res)
    }

    /// Returns the Content-Type header value if present.
    pub fn content_type(&self) -> Option<String> {
        self.response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    }
}

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
                    std::io::Error::new(std::io::ErrorKind::Unsupported, "Stream length unknown")
                })?;
                len.saturating_add_signed(delta)
            }
        };

        if new_pos != self.pos {
            debug!("RemoteReader seeking: {} -> {}", self.pos, new_pos);
            match Self::fetch_stream(&self.client, &self.url, new_pos) {
                Ok(res) => {
                    self.response = res;
                    self.pos = new_pos;
                }
                Err(e) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        e.to_string(),
                    ));
                }
            }
        }
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
