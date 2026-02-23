use std::io::{Read, Seek, SeekFrom};

use symphonia::core::io::MediaSource;

use crate::{
    audio::remote_reader::{BaseRemoteReader, create_client},
    common::types::AnyResult,
};

pub struct HttpReader {
    inner: BaseRemoteReader,
}

impl HttpReader {
    pub fn new(
        url: &str,
        local_addr: Option<std::net::IpAddr>,
        proxy: Option<crate::configs::HttpProxyConfig>,
    ) -> AnyResult<Self> {
        let user_agent = crate::common::http::default_user_agent();

        let client = create_client(user_agent, local_addr, proxy, None)?;
        let inner = BaseRemoteReader::new(client, url)?;

        Ok(Self { inner })
    }
}

impl Read for HttpReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.inner.read(buf)
    }
}

impl Seek for HttpReader {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(pos)
    }
}

impl MediaSource for HttpReader {
    fn is_seekable(&self) -> bool {
        self.inner.is_seekable()
    }

    fn byte_len(&self) -> Option<u64> {
        self.inner.byte_len()
    }

    // Explicitly delegate content_type if needed for probing
}

impl HttpReader {
    pub fn content_type(&self) -> Option<String> {
        self.inner.content_type()
    }
}
