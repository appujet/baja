/// Minimal HLS (HTTP Live Streaming) reader for Symphonia.
///
/// Implements [`std::io::Read`] / [`std::io::Seek`] / [`symphonia::core::io::MediaSource`]
/// by:
///   1. Fetching the master or media playlist
///   2. Resolving to the best audio-only variant (audio/mp4 or audio/mp3 codecs preferred)
///   3. Downloading segments sequentially into an in-memory ring buffer
///   4. Exposing them as a contiguous byte stream so Symphonia can probe/decode normally
///
/// HLS streams are not seekable in the traditional sense (no byte-level random access),
/// so [`is_seekable`] returns `false` and `byte_len` returns `None`.
use std::collections::HashMap;
use std::io::{self, Read, Seek, SeekFrom};
use symphonia::core::io::MediaSource;

// ─── Public entry point ───────────────────────────────────────────────────────

use crate::sources::youtube::cipher::YouTubeCipherManager;
use std::sync::Arc;

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
    /// Blocking reqwest client (same thread as Symphonia decode loop).
    client: reqwest::blocking::Client,
    /// YouTube cipher manager to resolve n-tokens on segments.
    cipher_manager: Option<Arc<YouTubeCipherManager>>,
    /// Original player page URL or player script URL for cipher resolution.
    player_url: Option<String>,
}

impl HlsReader {
    /// Blocking constructor.  Fetches and resolves the playlist synchronously, then
    /// returns a reader positioned at the start of the first segment.
    pub fn new(
        manifest_url: &str,
        local_addr: Option<std::net::IpAddr>,
        cipher_manager: Option<Arc<YouTubeCipherManager>>,
        player_url: Option<String>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let mut builder = reqwest::blocking::Client::builder()
            .user_agent(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) \
                 AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.0.0 Safari/537.36",
            )
            .timeout(std::time::Duration::from_secs(15));

        if let Some(ip) = local_addr {
            builder = builder.local_address(ip);
        }

        let client = builder.build()?;

        // Resolve master → media playlist if necessary.
        let (segment_urls, map_url) = resolve_playlist(&client, manifest_url)?;

        if segment_urls.is_empty() {
            return Err("HLS playlist contained no segments".into());
        }

        tracing::debug!(
            "HLS: resolved {} segments, map={:?} from {}",
            segment_urls.len(),
            map_url,
            manifest_url
        );

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

        // 1. Fetch initialization segment (map) if present.
        if let Some(map_res) = reader.map_url.clone() {
            let resolved = reader.resolve_resource(&map_res)?;
            fetch_segment_into(&reader.client, &resolved, &mut reader.buf)?;
            reader.map_fetched = true;
        }

        // 2. Fetch the first media segment.
        if !reader.pending.is_empty() {
            let first_res = reader.pending.remove(0);
            let resolved = reader.resolve_resource(&first_res)?;
            fetch_segment_into(&reader.client, &resolved, &mut reader.buf)?;
        }

        Ok(reader)
    }
}

// ─── Trait impls ─────────────────────────────────────────────────────────────

impl Read for HlsReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        // Fetch next segment if buffer is exhausted.
        while self.pos >= self.buf.len() {
            if self.pending.is_empty() {
                return Ok(0); // EOF
            }

            let res = self.pending.remove(0);
            // Compact the buffer (drop consumed bytes) before appending the new segment.
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

impl HlsReader {
    /// Resolve a Resource's URL if it contains a YouTube n-token.
    fn resolve_resource(
        &self,
        res: &Resource,
    ) -> Result<Resource, Box<dyn std::error::Error + Send + Sync>> {
        let mut resolved = res.clone();
        resolved.url = self.resolve_url_string(&res.url)?;
        Ok(resolved)
    }

    /// Resolve a segment URL string if it contains a YouTube n-token.
    fn resolve_url_string(
        &self,
        url: &str,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let (cipher, player_url) = match (&self.cipher_manager, &self.player_url) {
            (Some(c), Some(p)) => (c, p),
            _ => return Ok(url.to_string()),
        };

        // Extract n-token from /n/TOKEN/ or &n=TOKEN
        let n_token = if let Some(pos) = url.find("/n/") {
            let rest = &url[pos + 3..];
            rest.split('/').next()
        } else {
            url.split("&n=")
                .nth(1)
                .or_else(|| url.split("?n=").nth(1))
                .and_then(|s| s.split('&').next())
        };

        if let Some(n) = n_token {
            let handle = tokio::runtime::Handle::current();
            let cipher = cipher.clone();
            let url = url.to_string();
            let player_url = player_url.clone();
            let n = n.to_string();

            Ok(handle.block_on(async move {
                cipher.resolve_url(&url, &player_url, Some(&n), None).await
            })?)
        } else {
            Ok(url.to_string())
        }
    }
}

impl Seek for HlsReader {
    fn seek(&mut self, _pos: SeekFrom) -> io::Result<u64> {
        // HLS streams are not seekable. Symphonia will call this only if
        // is_seekable() returns true, which we prevent.
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

// ─── Internal helpers ─────────────────────────────────────────────────────────

/// Fetch one HTTP resource into `out`, following a single redirect if needed.
fn fetch_segment_into(
    client: &reqwest::blocking::Client,
    resource: &Resource,
    out: &mut Vec<u8>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing::debug!(
        "HLS: fetching {} (range={:?})",
        resource.url,
        resource.range
    );
    let mut req = client.get(&resource.url).header("Accept", "*/*");

    if let Some(range) = &resource.range {
        let end = range.offset + range.length - 1;
        req = req.header("Range", format!("bytes={}-{}", range.offset, end));
    }

    let mut res = req.send()?;

    if !res.status().is_success() {
        return Err(format!("HLS fetch failed {}: {}", res.status(), resource.url).into());
    }

    let n = res.copy_to(out)?;
    tracing::debug!("HLS: fetched {} bytes", n);
    Ok(())
}

/// Fetch and parse an HLS playlist, returning an ordered list of absolute segment URLs.
///
/// Handles two levels of indirection:
/// - **Master playlist** → picks the best audio-only variant by bandwidth, then recurses.
/// - **Media playlist** → returns all segment URLs in order.
fn resolve_playlist(
    client: &reqwest::blocking::Client,
    url: &str,
) -> Result<(Vec<Resource>, Option<Resource>), Box<dyn std::error::Error + Send + Sync>> {
    let text = fetch_text(client, url)?;
    let playlist = parse_m3u8(&text, url);

    match playlist {
        M3u8Playlist::Master {
            variants,
            audio_groups,
        } => {
            // Priority 1: Audio-only variants (no video).
            // Priority 2: Variants with an associated audio group.
            // Priority 3: Highest bandwidth variant.
            let best = variants
                .iter()
                .filter(|v| v.is_audio_only)
                .max_by_key(|v| v.bandwidth)
                .or_else(|| {
                    variants
                        .iter()
                        .filter(|v| v.audio_group.is_some())
                        .max_by_key(|v| v.bandwidth)
                })
                .or_else(|| variants.iter().max_by_key(|v| v.bandwidth));

            match best {
                Some(v) => {
                    // If the variant has an audio group, try to find a rendition URI.
                    if let Some(group_id) = &v.audio_group {
                        if let Some(group) = audio_groups.get(group_id) {
                            let rendition = group
                                .iter()
                                .find(|m| m.is_default)
                                .or_else(|| group.iter().find(|m| m.uri.is_some()))
                                .and_then(|m| m.uri.as_ref());

                            if let Some(uri) = rendition {
                                tracing::debug!(
                                    "HLS: selected audio group {} -> {}",
                                    group_id,
                                    uri
                                );
                                return resolve_playlist(client, uri);
                            }
                        }
                    }

                    tracing::debug!(
                        "HLS: selected variant bw={} codecs={:?} audio_only={} audio_group={:?} url={}",
                        v.bandwidth,
                        v.codecs,
                        v.is_audio_only,
                        v.audio_group,
                        v.url
                    );
                    resolve_playlist(client, &v.url)
                }
                None => Err("HLS master playlist has no variants".into()),
            }
        }
        M3u8Playlist::Media { segments, map } => Ok((segments, map)),
    }
}

fn fetch_text(
    client: &reqwest::blocking::Client,
    url: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let mut res = client
        .get(url)
        .header("Accept", "application/x-mpegURL, */*")
        .send()?;

    if !res.status().is_success() {
        return Err(format!("HLS playlist fetch failed {}: {}", res.status(), url).into());
    }

    let mut text = String::new();
    res.read_to_string(&mut text)?;
    Ok(text)
}

// ─── Minimal M3U8 parser ──────────────────────────────────────────────────────

struct Variant {
    url: String,
    bandwidth: u64,
    codecs: String,
    /// True when CODECS contains audio codec but no video codec (avc1/hvc1 etc.)
    is_audio_only: bool,
    /// AUDIO group identifier
    audio_group: Option<String>,
}

struct Media {
    _type: String,
    _group_id: String,
    uri: Option<String>,
    is_default: bool,
}

#[derive(Clone, Debug)]
struct ByteRange {
    length: u64,
    offset: u64,
}

#[derive(Clone, Debug)]
struct Resource {
    url: String,
    range: Option<ByteRange>,
}

enum M3u8Playlist {
    Master {
        variants: Vec<Variant>,
        audio_groups: HashMap<String, Vec<Media>>,
    },
    Media {
        segments: Vec<Resource>,
        map: Option<Resource>,
    },
}

/// Very small M3U8 parser — handles just enough of the spec for YouTube HLS.
fn parse_m3u8(text: &str, base_url: &str) -> M3u8Playlist {
    let lines: Vec<&str> = text.lines().map(str::trim).collect();

    // Decide master vs. media by presence of EXT-X-STREAM-INF.
    let is_master = lines.iter().any(|l| l.starts_with("#EXT-X-STREAM-INF"));

    if is_master {
        let mut variants = Vec::new();
        let mut audio_groups: HashMap<String, Vec<Media>> = HashMap::new();
        let mut i = 0;

        while i < lines.len() {
            let line = lines[i];
            if line.starts_with("#EXT-X-MEDIA") {
                let type_ = extract_attr_str(line, "TYPE").unwrap_or_default();
                let group_id = extract_attr_str(line, "GROUP-ID").unwrap_or_default();
                let uri = extract_attr_str(line, "URI").map(|u| resolve_url(base_url, &u));
                let is_default = extract_attr_str(line, "DEFAULT").as_deref() == Some("YES");

                if type_ == "AUDIO" && !group_id.is_empty() {
                    audio_groups
                        .entry(group_id.clone())
                        .or_default()
                        .push(Media {
                            _type: type_,
                            _group_id: group_id,
                            uri,
                            is_default,
                        });
                }
                i += 1;
            } else if line.starts_with("#EXT-X-STREAM-INF") {
                let bandwidth = extract_attr_u64(line, "BANDWIDTH").unwrap_or(0);
                let codecs = extract_attr_str(line, "CODECS").unwrap_or_default();
                let audio_group = extract_attr_str(line, "AUDIO");

                let has_audio =
                    codecs.contains("mp4a") || codecs.contains("opus") || codecs.contains("aac");
                let has_video = codecs.contains("avc1")
                    || codecs.contains("hvc1")
                    || codecs.contains("hev1")
                    || codecs.contains("dvh1")
                    || codecs.contains("vp09")
                    || codecs.contains("av01")
                    || codecs.contains("vp9")
                    || codecs.contains("av1")
                    || codecs.contains("vp8")
                    || codecs.contains("h264")
                    || codecs.contains("h265")
                    || codecs.contains("mp4v");

                let mut j = i + 1;
                while j < lines.len() && lines[j].starts_with('#') {
                    j += 1;
                }
                if j < lines.len() && !lines[j].is_empty() {
                    variants.push(Variant {
                        url: resolve_url(base_url, lines[j]),
                        bandwidth,
                        codecs: codecs.clone(),
                        is_audio_only: has_audio && !has_video,
                        audio_group,
                    });
                }
                i = j + 1;
            } else {
                i += 1;
            }
        }

        tracing::debug!("HLS: found {} variants", variants.len());
        for v in &variants {
            tracing::debug!(
                "  Variant: bw={} codecs={:?} audio_only={} audio_group={:?}",
                v.bandwidth,
                v.codecs,
                v.is_audio_only,
                v.audio_group
            );
        }

        return M3u8Playlist::Master {
            variants,
            audio_groups,
        };
    }

    // ── Media playlist ────────────────────────────────────────────────────────
    let mut segments = Vec::new();
    let mut map = None;
    let mut next_offset = 0u64;
    let mut pending_range: Option<ByteRange> = None;

    for i in 0..lines.len() {
        let line = lines[i];
        if line.starts_with("#EXT-X-MAP") {
            if let Some(url) = extract_attr_str(line, "URI").map(|u| resolve_url(base_url, &u)) {
                let range = extract_attr_str(line, "BYTERANGE").map(|r| parse_byte_range(&r, 0));
                map = Some(Resource { url, range });
            }
        } else if line.starts_with("#EXT-X-BYTERANGE:") {
            let r = parse_byte_range(&line[17..], next_offset);
            next_offset = r.offset + r.length;
            pending_range = Some(r);
        } else if line.starts_with("#EXTINF:") {
            let mut j = i + 1;
            while j < lines.len() && lines[j].starts_with('#') {
                if lines[j].starts_with("#EXT-X-BYTERANGE:") {
                    let r = parse_byte_range(&lines[j][17..], next_offset);
                    next_offset = r.offset + r.length;
                    pending_range = Some(r);
                }
                j += 1;
            }
            if j < lines.len() {
                segments.push(Resource {
                    url: resolve_url(base_url, lines[j]),
                    range: pending_range.take(),
                });
            }
        }
    }
    M3u8Playlist::Media { segments, map }
}

fn parse_byte_range(attr: &str, last_end_offset: u64) -> ByteRange {
    // Format: "length[@offset]"
    let attr = attr.trim().trim_matches('"');
    let parts: Vec<&str> = attr.split('@').collect();
    let length = parts[0].trim().parse::<u64>().unwrap_or(0);
    let offset = if parts.len() > 1 {
        parts[1].trim().parse::<u64>().unwrap_or(0)
    } else {
        last_end_offset
    };
    ByteRange { length, offset }
}

// ─── Tiny attribute parsing helpers ──────────────────────────────────────────

fn extract_attr_u64(line: &str, key: &str) -> Option<u64> {
    extract_attr_str(line, key)?.parse().ok()
}

fn extract_attr_str(line: &str, key: &str) -> Option<String> {
    let key_eq = format!("{}=", key);
    // Attributes follow #TAG: or a comma
    let pos = line
        .find(&format!(":{}", key_eq))
        .map(|p| p + 1)
        .or_else(|| line.find(&format!(",{}", key_eq)).map(|p| p + 1))?;

    let rest = &line[pos + key_eq.len()..];

    if rest.starts_with('"') {
        let end = rest[1..].find('"')?;
        Some(rest[1..1 + end].to_string())
    } else {
        let end = rest.find(',').unwrap_or(rest.len());
        Some(rest[..end].trim().to_string())
    }
}

/// Resolve a (possibly relative) segment/variant URL against the playlist base URL.
fn resolve_url(base: &str, maybe_relative: &str) -> String {
    if maybe_relative.starts_with("http://") || maybe_relative.starts_with("https://") {
        return maybe_relative.to_string();
    }

    // Absolute path → replace host + path.
    if maybe_relative.starts_with('/') {
        if let Some(scheme_end) = base.find("://") {
            let host_start = scheme_end + 3;
            let host_end = base[host_start..]
                .find('/')
                .map(|p| host_start + p)
                .unwrap_or(base.len());
            return format!("{}{}", &base[..host_end], maybe_relative);
        }
    }

    // Relative path → strip last path component from base and append.
    let base_dir = base.rfind('/').map(|i| &base[..=i]).unwrap_or(base);
    format!("{}{}", base_dir, maybe_relative)
}
