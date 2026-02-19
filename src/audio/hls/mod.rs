pub mod fetcher;
pub mod parser;
pub mod resolver;
pub mod types;
pub mod utils;

use self::fetcher::fetch_segment_into;
use self::resolver::{resolve_playlist, resolve_url_string};
use self::types::Resource;
use crate::sources::youtube::cipher::YouTubeCipherManager;
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::Arc;
use symphonia::core::io::MediaSource;

pub struct HlsReader {
    /// Accumulated bytes from already-downloaded segments.
    buf: Vec<u8>,
    /// Read cursor inside `buf`.
    pos: usize,
    /// Optional initialization segment (EXT-X-MAP) URL.
    map_url: Option<Resource>,
    /// Whether the map segment has already been fetched and prepended to `buf`.
    map_fetched: bool,
    /// Remaining segment resources that still need to be fetched.
    pending: Vec<Resource>,
    /// Blocking reqwest client.
    client: reqwest::blocking::Client,
    /// YouTube cipher manager to resolve n-tokens on segments.
    cipher_manager: Option<Arc<YouTubeCipherManager>>,
    /// Original player page URL.
    player_url: Option<String>,
}

impl HlsReader {
    pub fn new(
        manifest_url: &str,
        local_addr: Option<std::net::IpAddr>,
        cipher_manager: Option<Arc<YouTubeCipherManager>>,
        player_url: Option<String>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let mut builder = reqwest::blocking::Client::builder()
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.0.0 Safari/537.36")
            .timeout(std::time::Duration::from_secs(15));

        if let Some(ip) = local_addr {
            builder = builder.local_address(ip);
        }

        let client = builder.build()?;
        let (segment_urls, map_url) = resolve_playlist(&client, manifest_url)?;

        if segment_urls.is_empty() {
            return Err("HLS playlist contained no segments".into());
        }

        let mut reader = Self {
            buf: Vec::with_capacity(512 * 1024),
            pos: 0,
            map_url,
            map_fetched: false,
            pending: segment_urls,
            client,
            cipher_manager,
            player_url,
        };

        // Fetch initialization segment if present.
        if let Some(map_res) = reader.map_url.clone() {
            let resolved = reader.resolve_resource(&map_res)?;
            fetch_segment_into(&reader.client, &resolved, &mut reader.buf)?;
            reader.map_fetched = true;
        }

        // Fetch the first media segment.
        if !reader.pending.is_empty() {
            let first_res = reader.pending.remove(0);
            let resolved = reader.resolve_resource(&first_res)?;
            fetch_segment_into(&reader.client, &resolved, &mut reader.buf)?;
        }

        Ok(reader)
    }

    fn resolve_resource(
        &self,
        res: &Resource,
    ) -> Result<Resource, Box<dyn std::error::Error + Send + Sync>> {
        let mut resolved = res.clone();
        resolved.url = resolve_url_string(&res.url, &self.cipher_manager, &self.player_url)?;
        Ok(resolved)
    }
}

impl Read for HlsReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        while self.pos >= self.buf.len() {
            if self.pending.is_empty() {
                return Ok(0);
            }

            let res = self.pending.remove(0);
            self.buf.clear();
            self.pos = 0;

            let resolved = self.resolve_resource(&res).map_err(|e| {
                io::Error::new(
                    io::ErrorKind::Other,
                    format!("Segment resolution failed: {}", e),
                )
            })?;

            fetch_segment_into(&self.client, &resolved, &mut self.buf)
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
        }

        let available = &self.buf[self.pos..];
        let n = out.len().min(available.len());
        out[..n].copy_from_slice(&available[..n]);
        self.pos += n;
        Ok(n)
    }
}

impl Seek for HlsReader {
    fn seek(&mut self, _pos: SeekFrom) -> io::Result<u64> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "HLS streams are not seekable",
        ))
    }
}

impl MediaSource for HlsReader {
    fn is_seekable(&self) -> bool {
        false
    }
    fn byte_len(&self) -> Option<u64> {
        None
    }
}
