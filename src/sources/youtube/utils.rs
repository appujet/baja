use std::sync::Arc;

use symphonia::core::io::MediaSource;

use crate::{
    common::types::AudioFormat,
    sources::{
        http::reader::HttpReader,
        youtube::{cipher::YouTubeCipherManager, hls::HlsReader, reader::YoutubeReader},
    },
};

pub const DEFAULT_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/119.0.0.0 Safari/537.36";

pub fn detect_audio_kind(url: &str, is_hls: bool) -> AudioFormat {
    if is_hls {
        AudioFormat::Aac
    } else {
        AudioFormat::from_url(url)
    }
}

pub fn create_reader(
    url: &str,
    client_name: &str,
    local_addr: Option<std::net::IpAddr>,
    proxy: Option<crate::configs::HttpProxyConfig>,
    _cipher_manager: Arc<YouTubeCipherManager>,
) -> AnyResult<Box<dyn MediaSource>> {
    if url.contains(".m3u8") || url.contains("/playlist") {
        Ok(Box::new(HlsReader::new(
            url,
            local_addr,
            Some(_cipher_manager),
            None,
            proxy,
        )?))
    } else if client_name == "TV" {
        Ok(Box::new(YoutubeReader::new(url, local_addr, proxy)?))
    } else {
        Ok(Box::new(HttpReader::new(url, local_addr, proxy)?))
    }
}

type AnyResult<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
