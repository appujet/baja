use std::io::{Read, Seek, SeekFrom};

use symphonia::core::io::MediaSource;

use crate::{
    audio::source::{HttpSource, create_client},
    common::types::AnyResult,
};

pub struct MixcloudReader {
    inner: HttpSource,
}

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

impl MixcloudReader {
    pub fn new(url: &str, local_addr: Option<std::net::IpAddr>) -> AnyResult<Self> {
        let client = create_client(USER_AGENT.to_string(), local_addr, None, None)?;
        let inner = HttpSource::new(client, url)?;

        Ok(Self { inner })
    }
}

impl Read for MixcloudReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.inner.read(buf)
    }
}

impl Seek for MixcloudReader {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(pos)
    }
}

impl MediaSource for MixcloudReader {
    fn is_seekable(&self) -> bool {
        self.inner.is_seekable()
    }

    fn byte_len(&self) -> Option<u64> {
        self.inner.byte_len()
    }
}
