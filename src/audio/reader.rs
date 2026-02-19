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
            tracing::debug!("Configuring proxy for RemoteReader: {}", proxy_config.url);
            if let Ok(proxy_obj) = reqwest::Proxy::all(&proxy_config.url) {
                let mut proxy_obj = proxy_obj;
                if let (Some(username), Some(password)) = (proxy_config.username, proxy_config.password)
                {
                    proxy_obj = proxy_obj.basic_auth(&username, &password);
                }
                builder = builder.proxy(proxy_obj);
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
