use std::io::{Read, Seek, SeekFrom};
use symphonia::core::io::MediaSource;

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
        proxy: Option<crate::configs::HttpProxyConfig>,
    ) -> Result<Self, reqwest::Error> {
        let mut builder = reqwest::blocking::Client::builder()
            .user_agent(crate::common::http::HttpClient::random_user_agent())
            .timeout(std::time::Duration::from_secs(10));

        if let Some(ip) = local_addr {
            builder = builder.local_address(ip);
        }

        if let Some(proxy_config) = proxy {
            if let Some(url) = &proxy_config.url {
                tracing::debug!("Configuring proxy for RemoteReader: {}", url);
                if let Ok(proxy_obj) = reqwest::Proxy::all(url) {
                    let mut proxy_obj = proxy_obj;
                    if let (Some(username), Some(password)) = (proxy_config.username, proxy_config.password)
                    {
                        proxy_obj = proxy_obj.basic_auth(&username, &password);
                    }
                    builder = builder.proxy(proxy_obj);
                }
            }
        }

        let client = builder.build()?;
        let response = client.get(url).send()?;
        let len = response.content_length();
        let pos = 0;

        Ok(Self {
            url: url.to_string(),
            client,
            response,
            pos,
            len: len.map(|l| l as u64),
        })
    }
    pub fn content_type(&self) -> Option<String> {
        self.response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
    }
}

impl Read for RemoteReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self.response.read(buf) {
            Ok(n) => {
                self.pos += n as u64;
                Ok(n)
            }
            Err(e) => Err(e),
        }
    }
}

impl Seek for RemoteReader {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        let new_pos = match pos {
            SeekFrom::Start(p) => p,
            SeekFrom::Current(p) => (self.pos as i64 + p) as u64,
            SeekFrom::End(p) => {
                if let Some(len) = self.len {
                    (len as i64 + p) as u64
                } else {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::Unsupported,
                        "Unknown length",
                    ));
                }
            }
        };

        if new_pos == self.pos {
            return Ok(self.pos);
        }

        // Perform range request
        let range = format!("bytes={}-", new_pos);
        match self.client.get(&self.url).header("Range", range).send() {
            Ok(resp) => {
                self.response = resp;
                self.pos = new_pos;
                Ok(self.pos)
            }
            Err(e) => Err(std::io::Error::new(std::io::ErrorKind::Other, e)),
        }
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

pub fn create_media_source(
    playback_url: &str,
    local_addr: Option<std::net::IpAddr>,
    proxy: Option<crate::configs::HttpProxyConfig>,
) -> Result<Box<dyn MediaSource>, Box<dyn std::error::Error + Send + Sync>> {
    if playback_url.starts_with("deezer_encrypted:") {
        let prefix_len = "deezer_encrypted:".len();
        if let Some(rest) = playback_url.get(prefix_len..) {
            if let Some(colon_pos) = rest.find(':') {
                let track_id = &rest[..colon_pos];
                let real_url = &rest[colon_pos + 1..];
                
                let config = crate::configs::base::Config::load().unwrap_or_default();
                let master_key = config.deezer.as_ref()
                    .and_then(|c| c.master_decryption_key.clone())
                    .unwrap_or_default();
                
                return Ok(Box::new(crate::audio::deezer_reader::DeezerReader::new(
                    real_url,
                    track_id,
                    &master_key,
                    local_addr,
                    proxy,
                )?));
            } else {
                 return Err(Box::new(std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("Malformed Deezer URL: {}", playback_url))));
            }
        } else {
             return Err(Box::new(std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("Malformed Deezer URL: {}", playback_url))));
        }
    } else {
        Ok(Box::new(RemoteReader::new(
            playback_url,
            local_addr,
            proxy,
        )?))
    }
}
