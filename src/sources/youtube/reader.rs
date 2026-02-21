use std::io::{Read, Seek, SeekFrom};

use symphonia::core::io::MediaSource;

use super::ua::get_youtube_ua;
use crate::{
  audio::remote_reader::{create_client, segmented::SegmentedRemoteReader},
  common::types::AnyResult,
};

pub struct YoutubeReader {
  inner: SegmentedRemoteReader,
}

impl YoutubeReader {
  pub fn new(
    url: &str,
    local_addr: Option<std::net::IpAddr>,
    proxy: Option<crate::configs::HttpProxyConfig>,
  ) -> AnyResult<Self> {
    let user_agent = get_youtube_ua(url)
      .map(str::to_string)
      .unwrap_or_else(crate::common::http::HttpClient::default_user_agent);

    let client = create_client(user_agent, local_addr, proxy, None)?;
    let inner = SegmentedRemoteReader::new(client, url)?;

    Ok(Self { inner })
  }
}

impl Read for YoutubeReader {
  fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
    self.inner.read(buf)
  }
}

impl Seek for YoutubeReader {
  fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
    self.inner.seek(pos)
  }
}

impl MediaSource for YoutubeReader {
  fn is_seekable(&self) -> bool {
    self.inner.is_seekable()
  }

  fn byte_len(&self) -> Option<u64> {
    self.inner.byte_len()
  }
}
