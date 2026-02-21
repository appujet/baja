use crate::audio::remote_reader::{BaseRemoteReader, create_client};
use crate::common::types::AnyResult;
use std::io::{Read, Seek, SeekFrom};
use symphonia::core::io::MediaSource;

pub struct SoundCloudReader {
  inner: BaseRemoteReader,
}

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36";

impl SoundCloudReader {
  pub fn new(
    url: &str,
    local_addr: Option<std::net::IpAddr>,
    proxy: Option<crate::configs::HttpProxyConfig>,
  ) -> AnyResult<Self> {
    let client = create_client(USER_AGENT.to_string(), local_addr, proxy, None)?;
    let inner = BaseRemoteReader::new(client, url)?;

    Ok(Self { inner })
  }
}

impl Read for SoundCloudReader {
  fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
    self.inner.read(buf)
  }
}

impl Seek for SoundCloudReader {
  fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
    self.inner.seek(pos)
  }
}

impl MediaSource for SoundCloudReader {
  fn is_seekable(&self) -> bool {
    self.inner.is_seekable()
  }

  fn byte_len(&self) -> Option<u64> {
    self.inner.byte_len()
  }
}
