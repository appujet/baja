use std::{
    collections::HashSet,
    io::{self, Read, Seek, SeekFrom},
    net::IpAddr,
};

use symphonia::core::io::MediaSource;

use crate::{
    audio::processor::{AudioProcessor, DecoderCommand},
    common::types::AudioFormat,
    config::HttpProxyConfig,
    sources::{
        plugin::PlayableTrack,
        youtube::hls::{
            fetcher::fetch_segment_into,
            resolver::fetch_text,
            ts_demux::extract_adts_from_ts,
            types::Resource,
            utils::resolve_url,
        },
    },
};

pub struct TwitchTrack {
    pub stream_url: String,
    pub local_addr: Option<IpAddr>,
    pub proxy: Option<HttpProxyConfig>,
}

struct LiveHlsReader {
    chunk_rx: flume::Receiver<Vec<u8>>,
    current: Vec<u8>,
    pos: usize,
}

impl LiveHlsReader {
    fn new(
        manifest_url: String,
        local_addr: Option<IpAddr>,
        proxy: Option<HttpProxyConfig>,
        handle: tokio::runtime::Handle,
    ) -> Self {
        let (chunk_tx, chunk_rx) = flume::bounded::<Vec<u8>>(16);

        std::thread::Builder::new()
            .name("twitch-live-hls".into())
            .spawn(move || {
                let _guard = handle.enter();

                let mut builder = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(15));

                if let Some(ip) = local_addr {
                    builder = builder.local_address(ip);
                }

                if let Some(ref cfg) = proxy && let Some(ref url) = cfg.url {
                    if let Ok(mut p) = reqwest::Proxy::all(url) {
                        if let (Some(u), Some(pw)) = (&cfg.username, &cfg.password) {
                            p = p.basic_auth(u, pw);
                        }
                        builder = builder.proxy(p);
                    }
                }

                let client = match builder.build() {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::error!("Twitch live HLS: client build failed: {e}");
                        return;
                    }
                };

                let mut seen: HashSet<String> = HashSet::new();

                loop {
                    let text = match handle.block_on(fetch_text(&client, &manifest_url)) {
                        Ok(t) => t,
                        Err(e) => {
                            tracing::warn!("Twitch: live playlist refresh failed: {e}");
                            std::thread::sleep(std::time::Duration::from_secs(2));
                            continue;
                        }
                    };

                    let (segments, target_duration) =
                        parse_live_playlist(&text, &manifest_url, &mut seen);

                    for seg in segments {
                        let mut raw = Vec::new();
                        if let Err(e) =
                            handle.block_on(fetch_segment_into(&client, &seg, &mut raw))
                        {
                            tracing::warn!("Twitch: segment fetch error: {e}");
                            continue;
                        }

                        let payload = if raw.first() == Some(&0x47) {
                            let adts = extract_adts_from_ts(&raw);
                            if adts.is_empty() { raw } else { adts }
                        } else {
                            raw
                        };

                        if chunk_tx.send(payload).is_err() {
                            return;
                        }
                    }

                    let wait = (target_duration / 2.0).max(1.0);
                    std::thread::sleep(std::time::Duration::from_secs_f64(wait));
                }
            })
            .expect("failed to spawn twitch live HLS thread");

        Self {
            chunk_rx,
            current: Vec::new(),
            pos: 0,
        }
    }
}

fn parse_live_playlist(
    text: &str,
    base_url: &str,
    seen: &mut HashSet<String>,
) -> (Vec<Resource>, f64) {
    let mut segments = Vec::new();
    let mut target_duration = 6.0f64;
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];

        if let Some(rest) = line.strip_prefix("#EXT-X-TARGETDURATION:") {
            if let Ok(d) = rest.trim().parse::<f64>() {
                target_duration = d;
            }
        } else if line.starts_with("#EXTINF:") {
            let duration = line
                .strip_prefix("#EXTINF:")
                .and_then(|r| r.split(',').next())
                .and_then(|d| d.trim().parse::<f64>().ok());

            let mut j = i + 1;
            while j < lines.len() && lines[j].starts_with('#') {
                j += 1;
            }
            if j < lines.len() && !lines[j].is_empty() {
                let url = resolve_url(base_url, lines[j]);
                if seen.insert(url.clone()) {
                    segments.push(Resource { url, range: None, duration });
                }
            }
            i = j + 1;
            continue;
        }

        i += 1;
    }

    (segments, target_duration)
}

impl Read for LiveHlsReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            if self.pos < self.current.len() {
                let n = buf.len().min(self.current.len() - self.pos);
                buf[..n].copy_from_slice(&self.current[self.pos..self.pos + n]);
                self.pos += n;
                return Ok(n);
            }

            match self.chunk_rx.recv_timeout(std::time::Duration::from_millis(500)) {
                Ok(chunk) => {
                    self.current = chunk;
                    self.pos = 0;
                }
                Err(flume::RecvTimeoutError::Timeout) => continue,
                Err(flume::RecvTimeoutError::Disconnected) => return Ok(0),
            }
        }
    }
}

impl Seek for LiveHlsReader {
    fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "live streams are not seekable",
        ))
    }
}

impl MediaSource for LiveHlsReader {
    fn is_seekable(&self) -> bool {
        false
    }
    fn byte_len(&self) -> Option<u64> {
        None
    }
}

impl PlayableTrack for TwitchTrack {
    fn start_decoding(
        &self,
        config: crate::config::player::PlayerConfig,
    ) -> (
        flume::Receiver<crate::audio::buffer::PooledBuffer>,
        flume::Sender<DecoderCommand>,
        flume::Receiver<String>,
        Option<flume::Receiver<std::sync::Arc<Vec<u8>>>>,
    ) {
        let (tx, rx) = flume::bounded((config.buffer_duration_ms / 20) as usize);
        let (cmd_tx, cmd_rx) = flume::unbounded::<DecoderCommand>();
        let (err_tx, err_rx) = flume::bounded::<String>(1);

        let url = self.stream_url.clone();
        let local_addr = self.local_addr;
        let proxy = self.proxy.clone();
        let handle = tokio::runtime::Handle::current();

        std::thread::spawn(move || {
            let _guard = handle.enter();
            let reader = Box::new(LiveHlsReader::new(url, local_addr, proxy, handle.clone()))
                as Box<dyn MediaSource>;

            match AudioProcessor::new(
                reader,
                Some(AudioFormat::Aac),
                tx,
                cmd_rx,
                Some(err_tx.clone()),
                config,
            ) {
                Ok(mut processor) => {
                    if let Err(e) = processor.run() {
                        tracing::error!("Twitch HLS processor error: {e}");
                    }
                }
                Err(e) => {
                    tracing::error!("Twitch HLS processor init failed: {e}");
                    let _ = err_tx.send(format!("Failed to initialize processor: {e}"));
                }
            }
        });

        (rx, cmd_tx, err_rx, None)
    }
}
