use futures::StreamExt;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

use base64::{
    Engine as _, engine::general_purpose::STANDARD as B64,
    engine::general_purpose::URL_SAFE as B64URL,
};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tokio_util::bytes::Bytes;

use crate::common::types::AnyResult;

pub(crate) fn decode_po_token(s: &str) -> Option<Vec<u8>> {
    let mut s = s.replace('-', "+").replace('_', "/");
    let mod_len = s.len() % 4;
    if mod_len != 0 {
        s.push_str(&"=".repeat(4 - mod_len));
    }
    B64.decode(&s).ok()
}

use super::config::{SabrConfig, SabrFormat};
use super::proto::{
    self, EncodedBufferedRange, MediaHeaderMsg, UMP_FORMAT_INITIALIZATION_METADATA, UMP_MEDIA,
    UMP_MEDIA_END, UMP_MEDIA_HEADER, UMP_NEXT_REQUEST_POLICY, UMP_RELOAD_PLAYER_RESPONSE,
    UMP_SABR_CONTEXT_SENDING_POLICY, UMP_SABR_CONTEXT_UPDATE, UMP_SABR_ERROR, UMP_SABR_REDIRECT,
    UMP_STREAM_PROTECTION_STATUS,
};

#[derive(Debug, Clone)]
pub enum SabrEvent {
    Stall,
    Finished,
    Error(String),
}

#[derive(Debug, Clone)]
pub enum SabrCommand {
    UpdateSession {
        server_abr_url: String,
        ustreamer_config: String,
        po_token: Option<Vec<u8>>,
        playback_cookie: Option<Vec<u8>>,
    },
    Seek(u64),
}

const MIN_REQUEST_INTERVAL_MS: u64 = 500;
const MAX_NO_MEDIA_STREAK: u32 = 12;

/// Key for a format: "itag:xtags"
fn format_key(itag: i32, xtags: Option<&str>) -> String {
    match xtags {
        Some(x) if !x.is_empty() => format!("{}:{}", itag, x),
        _ => format!("{}:", itag),
    }
}

/// Decoded segment tracking
#[allow(dead_code)]
struct SegmentMeta {
    itag: i32,
    sequence_number: u32,
    duration_ms: u64,
    start_ms: u64,
    timescale: u32,
    last_modified: String,
    xtags: Option<String>,
}

/// A pending partial segment being received across multiple chunks
struct PendingSegment {
    format_key: String,
    sequence_number: u32,
    media_header: MediaHeaderMsg,
}

/// SABR context blob received from the server
#[derive(Clone)]
struct SabrContext {
    context_type: i32,
    value: Vec<u8>,
}

struct FormatInitMeta {
    end_segment_number: Option<u32>,
    #[allow(dead_code)]
    mime_type: String,
}

/// The core SABR polling state engine.
struct SabrInner {
    http: reqwest::Client,
    video_id: String,
    server_abr_url: String,
    ustreamer_config: Vec<u8>,
    visitor_data: Option<String>,
    po_token: Option<Vec<u8>>,
    client_name_id: i32,
    client_version: String,
    user_agent: String,
    selected_format: SabrFormat,

    // Session state
    request_number: u32,
    bandwidth_estimate: u64,
    total_downloaded_ms: u64,
    next_backoff_ms: u64,
    no_media_streak: u32,
    playback_cookie: Option<Vec<u8>>,
    initialized_formats: HashMap<String, FormatInitMeta>,
    // headerId -> pending segment
    partial_segments: HashMap<u8, PendingSegment>,
    // formatKey -> vec of segment metas seen so far (for bufferedRanges)
    completed_segments: HashMap<String, Vec<SegmentMeta>>,
    // sequence counters for each itag
    format_sequence_counters: HashMap<i32, u32>,
    // SABR contexts
    sabr_contexts: HashMap<i32, SabrContext>,
    active_context_types: HashSet<i32>,

    // Streaming parser
    ump_parser: proto::UmpStreamParser,
    cached_ranges: Vec<proto::EncodedBufferedRange>,
    stream_finished: bool,
    recovery_pending: bool,
    start_offset_ms: u64,
    session_start_time: Option<std::time::Instant>,

    tx: mpsc::Sender<Bytes>,
    event_tx: flume::Sender<SabrEvent>,
    cmd_rx: flume::Receiver<SabrCommand>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PollResult {
    /// Request succeeded, loop should continue
    Continue {
        saw_media: bool,
        bytes: usize,
        elapsed_ms: u128,
    },
    /// Server returned 204 No Content, loop should stop
    Stop,
}

impl SabrInner {
    fn new(
        config: &SabrConfig,
        selected: SabrFormat,
        tx: mpsc::Sender<Bytes>,
        event_tx: flume::Sender<SabrEvent>,
        cmd_rx: flume::Receiver<SabrCommand>,
    ) -> Self {
        let ustreamer_config = B64
            .decode(&config.ustreamer_config)
            .or_else(|_| B64URL.decode(&config.ustreamer_config))
            .unwrap_or_else(|e| {
                tracing::warn!(
                    "SABR[{}]: failed to decode videoPlaybackUstreamerConfig: {}",
                    config.server_abr_url,
                    e
                );
                Vec::new()
            });

        let po_token = config.po_token.as_deref().and_then(decode_po_token);

        let http = reqwest::Client::builder()
            .user_agent(config.user_agent.as_str())
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_default();

        Self {
            http,
            video_id: String::new(), // set below
            server_abr_url: config.server_abr_url.clone(),
            ustreamer_config,
            visitor_data: config.visitor_data.clone(),
            po_token,
            client_name_id: config.client_name_id,
            client_version: config.client_version.clone(),
            user_agent: config.user_agent.clone(),
            selected_format: selected,
            request_number: 0,
            bandwidth_estimate: 1_000_000,
            total_downloaded_ms: config.start_time_ms,
            next_backoff_ms: 0,
            no_media_streak: 0,
            playback_cookie: None,
            initialized_formats: HashMap::new(),
            partial_segments: HashMap::new(),
            completed_segments: HashMap::new(),
            format_sequence_counters: HashMap::new(),
            sabr_contexts: HashMap::new(),
            active_context_types: HashSet::new(),
            ump_parser: proto::UmpStreamParser::new(),
            cached_ranges: Vec::new(),
            recovery_pending: false,
            stream_finished: false,
            start_offset_ms: config.start_time_ms,
            session_start_time: None,
            tx,
            event_tx,
            cmd_rx,
        }
    }

    fn virtual_player_time_ms(&mut self) -> u64 {
        let base = self.start_offset_ms;
        match self.session_start_time {
            Some(start) => base + start.elapsed().as_millis() as u64,
            None => {
                self.session_start_time = Some(std::time::Instant::now());
                base
            }
        }
    }

    fn check_commands(&mut self) {
        while let Ok(cmd) = self.cmd_rx.try_recv() {
            match cmd {
                SabrCommand::UpdateSession {
                    server_abr_url,
                    ustreamer_config,
                    po_token,
                    playback_cookie,
                } => {
                    tracing::trace!("SABR[{}]: session updated via recovery", self.video_id);
                    self.server_abr_url = server_abr_url;

                    if let Ok(config) = B64
                        .decode(&ustreamer_config)
                        .or_else(|_| B64URL.decode(&ustreamer_config))
                    {
                        self.ustreamer_config = config;
                    }

                    if let Some(pt) = po_token {
                        self.po_token = Some(pt);
                    }

                    // Update playback cookie (or clear it if none provided)
                    if let Some(cookie) = playback_cookie {
                        self.playback_cookie = Some(cookie);
                    } else {
                        self.playback_cookie = None;
                    }

                    self.completed_segments.clear();
                    self.cached_ranges.clear();

                    self.request_number = 0;
                    self.no_media_streak = 0;
                    self.recovery_pending = false;
                    tracing::debug!(
                        "SABR[{}]: recovery complete, contexts cleared, rn reset to 0",
                        self.video_id
                    );
                }
                SabrCommand::Seek(ms) => {
                    tracing::trace!("SABR[{}]: fast seek to {}ms", self.video_id, ms);
                    self.total_downloaded_ms = ms;
                    self.start_offset_ms = ms;
                    self.session_start_time = None;
                    self.request_number = 0;
                    self.next_backoff_ms = 0;
                    self.no_media_streak = 0;
                    self.recovery_pending = false;
                    self.partial_segments.clear();
                    self.format_sequence_counters.clear();
                    self.completed_segments.clear();
                    self.cached_ranges.clear();
                    self.initialized_formats.clear();
                }
            }
        }
    }

    fn format_key(&self) -> String {
        format_key(
            self.selected_format.itag,
            self.selected_format.xtags.as_deref(),
        )
    }

    fn is_initialized(&self) -> bool {
        self.initialized_formats.contains_key(&self.format_key())
    }

    fn build_selected_format_ids(&self) -> Vec<(i32, String, Option<String>)> {
        if self.is_initialized() {
            vec![(
                self.selected_format.itag,
                self.selected_format.last_modified.clone(),
                self.selected_format.xtags.clone(),
            )]
        } else {
            vec![]
        }
    }

    fn build_buffered_ranges(&mut self) -> Vec<EncodedBufferedRange> {
        let key = self.format_key();

        // : only rebuild cachedBufferedRanges when new segments are available.
        // Otherwise, return the cached value.
        let has_new_segs = self
            .completed_segments
            .get(&key)
            .map(|s| !s.is_empty())
            .unwrap_or(false);

        if !has_new_segs {
            return self.cached_ranges.clone();
        }

        let segs = self.completed_segments.get(&key).unwrap();
        let duration_ms: u64 = segs.iter().map(|s| s.duration_ms).sum();
        let start = &segs[0];
        let end = segs.last().unwrap();
        let timescale = segs
            .iter()
            .find_map(|s| {
                if s.timescale > 0 {
                    Some(s.timescale)
                } else {
                    None
                }
            })
            .unwrap_or(1000);

        let range = EncodedBufferedRange {
            itag: self.selected_format.itag,
            last_modified: self.selected_format.last_modified.clone(),
            xtags: self.selected_format.xtags.clone(),
            start_time_ms: start.start_ms,
            duration_ms,
            start_segment_index: start.sequence_number,
            end_segment_index: end.sequence_number,
            timescale,
        };

        // Cache the range to send in subsequent requests until media is seen.
        self.cached_ranges = vec![range];
        // Clear raw segment list ).
        self.completed_segments.insert(key, Vec::new());

        self.cached_ranges.clone()
    }

    fn build_sabr_contexts(&self) -> Vec<(i32, Vec<u8>)> {
        self.sabr_contexts
            .values()
            .filter(|c| self.active_context_types.contains(&c.context_type))
            .map(|c| (c.context_type, c.value.clone()))
            .collect()
    }

    fn unsent_context_types(&self) -> Vec<i32> {
        self.sabr_contexts
            .values()
            .filter(|c| !self.active_context_types.contains(&c.context_type))
            .map(|c| c.context_type)
            .collect()
    }

    fn build_request_body(&mut self) -> Vec<u8> {
        let itag = self.selected_format.itag;
        let last_modified = self.selected_format.last_modified.clone();
        let xtags = self.selected_format.xtags.clone();
        let audio_track_id = self.selected_format.audio_track_id.clone();

        let buffered_ranges = self.build_buffered_ranges();
        let selected = self.build_selected_format_ids();
        let selected_refs: Vec<(i32, &str, Option<&str>)> = selected
            .iter()
            .map(|(i, lm, xt)| (*i, lm.as_str(), xt.as_deref()))
            .collect();

        let preferred = vec![(itag, last_modified.as_str(), xtags.as_deref())];

        let contexts = self.build_sabr_contexts();
        let unsent = self.unsent_context_types();

        let player_time_ms = self.total_downloaded_ms;
        let player_state = 1u64; // Always PLAYING (1). always sends 1n.

        let vec = proto::encode_video_playback_abr_request(
            self.bandwidth_estimate,
            player_time_ms,
            1, // AUDIO_ONLY
            audio_track_id.as_deref().unwrap_or(""),
            &selected_refs,
            &buffered_ranges,
            &preferred,
            &self.ustreamer_config,
            self.client_name_id,
            &self.client_version,
            self.po_token.as_deref(),
            self.playback_cookie.as_deref(),
            &contexts,
            &unsent,
            player_state,
        );

        vec
    }

    fn request_url(&mut self) -> String {
        let rn = self.request_number;
        self.request_number += 1;

        match reqwest::Url::parse(&self.server_abr_url) {
            Ok(mut url) => {
                url.query_pairs_mut()
                    .clear()
                    .extend_pairs(
                        reqwest::Url::parse(&self.server_abr_url)
                            .unwrap()
                            .query_pairs()
                            .filter(|(k, _)| k != "rn" && k != "alr" && k != "ump" && k != "srfvp"),
                    )
                    .append_pair("alr", "yes")
                    .append_pair("ump", "1")
                    .append_pair("srfvp", "1")
                    .append_pair("rn", &rn.to_string());
                url.to_string()
            }
            Err(_) => {
                // Fallback to string manipulation if URL parsing fails for some reason
                let base = &self.server_abr_url;
                let sep = if base.contains('?') { '&' } else { '?' };
                format!("{}{}alr=yes&ump=1&srfvp=1&rn={}", base, sep, rn)
            }
        }
    }

    async fn fetch_once(&mut self) -> AnyResult<PollResult> {
        if self.next_backoff_ms > 0 {
            let backoff = self.next_backoff_ms;
            tracing::debug!("SABR[{}]: backoff {}ms", self.video_id, backoff);
            sleep(Duration::from_millis(backoff)).await;
            self.next_backoff_ms = 0;
        }

        let t0 = std::time::Instant::now();
        let body = self.build_request_body();
        let url = self.request_url();

        let rn = self.request_number - 1;
        tracing::debug!(
            "SABR[{}]: request rn={} body={}B url={}",
            self.video_id,
            rn,
            body.len(),
            url
        );

        let mut req = self
            .http
            .post(&url)
            .header("content-type", "application/x-protobuf")
            .header("accept", "application/vnd.yt-ump")
            .header("origin", "https://www.youtube.com")
            .header(
                "referer",
                format!("https://www.youtube.com/watch?v={}", self.video_id),
            )
            .header("user-agent", &self.user_agent)
            .header("x-youtube-client-name", self.client_name_id.to_string())
            .header("x-youtube-client-version", &self.client_version);

        if let Some(vd) = &self.visitor_data {
            req = req.header("x-goog-visitor-id", vd);
        }

        let res = req.body(body).send().await?;
        let status = res.status();

        if status.as_u16() == 204 {
            tracing::debug!(
                "SABR[{}]: rn={} got 204 No Content → stream done",
                self.video_id,
                rn
            );
            return Ok(PollResult::Stop);
        }

        if !status.is_success() {
            let text = res.text().await.unwrap_or_default();
            return Err(format!("HTTP {}: {}", status, &text[..text.len().min(500)]).into());
        }

        let mut saw_media = false;
        let mut total_bytes = 0;
        let mut stream = res.bytes_stream();

        while let Some(chunk_res) = stream.next().await {
            let chunk = chunk_res?;
            total_bytes += chunk.len();

            let parts = self.ump_parser.push(&chunk);
            for (part_type, payload) in parts {
                if self.handle_part(part_type, &payload).await? {
                    saw_media = true;
                }
            }
        }

        tracing::trace!(
            "SABR[{}]: response rn={} bytes={}",
            self.video_id,
            rn,
            total_bytes
        );

        Ok(PollResult::Continue {
            saw_media,
            bytes: total_bytes,
            elapsed_ms: t0.elapsed().as_millis(),
        })
    }

    async fn handle_part(&mut self, part_type: u64, payload: &[u8]) -> AnyResult<bool> {
        match part_type {
            UMP_FORMAT_INITIALIZATION_METADATA => {
                self.handle_format_init(payload);
            }
            UMP_NEXT_REQUEST_POLICY => {
                self.handle_next_request_policy(payload);
            }
            UMP_SABR_ERROR => {
                let err = proto::decode_sabr_error(payload);
                return Err(format!("SABR error code={} type={}", err.code, err.error_type).into());
            }
            UMP_SABR_REDIRECT => {
                if let Some(new_url) = proto::decode_sabr_redirect(payload) {
                    tracing::debug!(
                        "SABR[{}]: redirect → {}",
                        self.video_id,
                        &new_url[..new_url.len().min(80)]
                    );
                    let sep = if new_url.contains('?') { '&' } else { '?' };
                    self.server_abr_url = format!("{}{}alr=yes&ump=1&srfvp=1", new_url, sep);
                }
            }
            UMP_RELOAD_PLAYER_RESPONSE => {
                tracing::debug!("SABR[{}]: server requested player reload", self.video_id);
                return Err("SABR reload requested by server".into());
            }
            UMP_SABR_CONTEXT_UPDATE => {
                if let Some(ctx) = proto::decode_sabr_context_update(payload) {
                    tracing::trace!(
                        "SABR[{}]: context update type={} len={} sendByDefault={}",
                        self.video_id,
                        ctx.context_type,
                        ctx.value.len(),
                        ctx.send_by_default
                    );
                    if ctx.send_by_default {
                        self.active_context_types.insert(ctx.context_type);
                    }
                    self.sabr_contexts.insert(
                        ctx.context_type,
                        SabrContext {
                            context_type: ctx.context_type,
                            value: ctx.value,
                        },
                    );
                }
            }
            UMP_SABR_CONTEXT_SENDING_POLICY => {
                let policy = proto::decode_sabr_context_sending_policy(payload);
                for t in policy.start_policy {
                    self.active_context_types.insert(t);
                }
                for t in policy.stop_policy {
                    self.active_context_types.remove(&t);
                }
                for t in policy.discard_policy {
                    self.active_context_types.remove(&t);
                    self.sabr_contexts.remove(&t);
                }
            }
            UMP_STREAM_PROTECTION_STATUS => {
                let status = proto::decode_stream_protection_status(payload);
                self.handle_stream_protection_status(status);
            }
            UMP_MEDIA_HEADER => {
                self.handle_media_header(payload);
            }
            UMP_MEDIA => {
                if self.handle_media(payload).await? {
                    return Ok(true);
                }
            }
            UMP_MEDIA_END => {
                self.handle_media_end(payload);
            }
            other => {
                tracing::trace!("SABR[{}]: unknown part type={}", self.video_id, other);
            }
        }
        Ok(false)
    }

    fn handle_format_init(&mut self, payload: &[u8]) {
        if let Some(m) = proto::decode_format_init_metadata(payload) {
            let key = format_key(m.itag, m.xtags.as_deref());
            tracing::trace!(
                "SABR[{}]: format init key={} end_seg={:?} mime={}",
                self.video_id,
                key,
                m.end_segment_number,
                m.mime_type
            );
            self.initialized_formats.insert(
                key,
                FormatInitMeta {
                    end_segment_number: m.end_segment_number,
                    mime_type: m.mime_type,
                },
            );
        }
    }

    fn handle_next_request_policy(&mut self, payload: &[u8]) {
        let p = proto::decode_next_request_policy(payload);
        tracing::trace!(
            "SABR[{}]: next request policy backoff={}ms cookieLen={}",
            self.video_id,
            p.backoff_ms,
            p.playback_cookie.as_ref().map(|c| c.len()).unwrap_or(0)
        );
        self.next_backoff_ms = p.backoff_ms;
        if let Some(cookie) = p.playback_cookie {
            self.playback_cookie = Some(cookie);
        }
    }

    fn handle_media_header(&mut self, payload: &[u8]) {
        if let Some(h) = proto::decode_media_header(payload) {
            let key = format_key(h.itag, h.xtags.as_deref());
            let seq = if h.is_init_seg {
                0u32
            } else if h.sequence_number > 0 {
                h.sequence_number
            } else {
                // FALLBACK: Uses formatSequenceCounters[itag]++
                self.format_sequence_counters
                    .get(&h.itag)
                    .cloned()
                    .unwrap_or(0)
                    + 1
            };

            if !h.is_init_seg {
                self.format_sequence_counters.insert(h.itag, seq);
            }

            // Compute duration from timeRange if durationMs == 0
            let duration_ms = if h.duration_ms > 0 {
                h.duration_ms
            } else if h.timescale > 0 && h.duration_ticks > 0 {
                (h.duration_ticks * 1000) / h.timescale as u64
            } else {
                0
            };

            let mut media_header = h;
            media_header.duration_ms = duration_ms;

            tracing::trace!(
                "SABR[{}]: media header id={} itag={} key={} seq={} init={} dur={}ms",
                self.video_id,
                media_header.header_id,
                media_header.itag,
                key,
                seq,
                media_header.is_init_seg,
                duration_ms
            );

            self.partial_segments.insert(
                media_header.header_id,
                PendingSegment {
                    format_key: key,
                    sequence_number: seq,
                    media_header,
                },
            );
        }
    }

    fn handle_stream_protection_status(&mut self, status: i32) {
        if status == 2 {
            tracing::debug!(
                "SABR[{}]: stream protection status=2 (limited playback). Signaling stall and resetting PO token.",
                self.video_id
            );
            self.po_token = None;
            self.recovery_pending = true;
            let _ = self.event_tx.send(SabrEvent::Stall);
        } else if status == 1 {
            tracing::warn!(
                "SABR[{}]: Stream Protection Status: 1 (Enabled)",
                self.video_id
            );
        } else {
            tracing::debug!(
                "SABR[{}]: stream protection status={}",
                self.video_id,
                status
            );
        }
    }

    async fn handle_media(&mut self, payload: &[u8]) -> AnyResult<bool> {
        if payload.is_empty() {
            return Ok(false);
        }
        let header_id = payload[0];
        let data = &payload[1..];

        // Only forward data for our selected format
        let selected_itag = self.selected_format.itag;
        let belongs_to_us = self
            .partial_segments
            .get(&header_id)
            .map(|s| s.media_header.itag == selected_itag)
            .unwrap_or(true); // if we have no header info yet, forward anyway

        if !belongs_to_us || data.is_empty() {
            return Ok(false);
        }

        // Send audio bytes downstream
        if let Err(e) = self.tx.send(Bytes::copy_from_slice(data)).await {
            tracing::debug!("SABR[{}]: channel closed: {}", self.video_id, e);
            return Err("channel closed".into());
        }
        Ok(true)
    }

    fn handle_media_end(&mut self, payload: &[u8]) {
        if payload.is_empty() {
            return;
        }
        let header_id = payload[0];

        let Some(pending) = self.partial_segments.remove(&header_id) else {
            return;
        };
        let h = &pending.media_header;

        let duration_ms = if h.duration_ms > 0 {
            h.duration_ms
        } else if h.timescale > 0 && h.duration_ticks > 0 {
            (h.duration_ticks * 1000) / h.timescale as u64
        } else {
            0
        };

        let start_ms = if h.start_ms > 0 {
            h.start_ms
        } else if h.timescale > 0 && h.start_ticks > 0 {
            (h.start_ticks * 1000) / h.timescale as u64
        } else {
            0
        };

        let end_ms = start_ms + duration_ms;
        if end_ms > self.total_downloaded_ms {
            self.total_downloaded_ms = end_ms;
        }

        tracing::debug!(
            "SABR[{}]: media end id={} itag={} seq={} dur={}ms total={}ms",
            self.video_id,
            header_id,
            h.itag,
            pending.sequence_number,
            duration_ms,
            self.total_downloaded_ms
        );

        // Track buffered ranges for follow-up requests
        let key = pending.format_key.clone();
        self.completed_segments
            .entry(key.clone())
            .or_default()
            .push(SegmentMeta {
                itag: h.itag,
                sequence_number: pending.sequence_number,
                duration_ms,
                start_ms,
                timescale: h.timescale,
                last_modified: h.last_modified.clone(),
                xtags: h.xtags.clone(),
            });

        // Check if we've received the final segment for our format
        if h.itag == self.selected_format.itag {
            if let Some(meta) = self.initialized_formats.get(&key) {
                if let Some(end_seg) = meta.end_segment_number {
                    if pending.sequence_number >= end_seg {
                        tracing::debug!(
                            "SABR[{}]: stream complete at segment {}/{}",
                            self.video_id,
                            pending.sequence_number,
                            end_seg
                        );
                        self.stream_finished = true;
                    }
                }
            }
        }
    }

    fn update_bandwidth_estimate(&mut self, bytes: usize, elapsed_ms: u64) {
        if elapsed_ms == 0 || bytes == 0 {
            return;
        }
        let measured_bps = (bytes as u64 * 8 * 1000) / elapsed_ms;
        self.bandwidth_estimate = (self.bandwidth_estimate * 3 + measured_bps) / 4;
    }
}


pub fn start_sabr_stream(
    video_id: String,
    config: SabrConfig,
) -> Option<(
    mpsc::Receiver<Bytes>,
    flume::Receiver<SabrEvent>,
    flume::Sender<SabrCommand>,
    JoinHandle<()>,
)> {
   
    let selected = match config.best_audio_format().cloned() {
        Some(fmt) => fmt,
        None => {
            tracing::warn!(
                "SABR[{}]: no audio formats in player response — cannot start stream",
                video_id
            );
            return None;
        }
    };

    let mime = selected.mime_type.clone();

    let (tx, rx) = mpsc::channel::<Bytes>(256);
    let (event_tx, event_rx) = flume::unbounded();
    let (cmd_tx, cmd_rx) = flume::unbounded();

    let mut inner = SabrInner::new(&config, selected, tx, event_tx, cmd_rx);
    inner.video_id = video_id.clone();

    tracing::info!(
        "SABR[{}]: starting stream itag={} mime={}",
        video_id,
        inner.selected_format.itag,
        mime
    );

    let handle = tokio::spawn(async move {
        let mut last_request_at = std::time::Instant::now();

        loop {
            inner.check_commands();
            if inner.stream_finished {
                tracing::debug!("SABR[{}]: stream finished", inner.video_id);
                let _ = inner.event_tx.send(SabrEvent::Finished);
                break;
            }

            if inner.tx.is_closed() {
                tracing::debug!(
                    "SABR[{}]: audio channel closed — exiting poll loop",
                    inner.video_id
                );
                break;
            }

            if inner.recovery_pending {
                sleep(Duration::from_millis(200)).await;
                inner.check_commands();
                continue;
            }

            let vpt = inner.virtual_player_time_ms();
            let downloaded = inner.total_downloaded_ms;
            const TARGET_BUFFER_MS: u64 = 30_000;
            if downloaded > vpt + TARGET_BUFFER_MS {
                tracing::trace!(
                    "SABR[{}]: ahead by {}ms, waiting (target={}ms)",
                    inner.video_id,
                    downloaded - vpt,
                    TARGET_BUFFER_MS,
                );
                sleep(Duration::from_millis(250)).await;
                continue;
            }

            let rn = inner.request_number;
            if rn > 0 {
                tracing::debug!(
                    "SABR[{}]: Tracking: downloaded={}ms virtualPlayerTime={}ms",
                    inner.video_id,
                    downloaded,
                    vpt,
                );
            }

            // Respect minimum request interval
            let elapsed = last_request_at.elapsed().as_millis() as u64;
            if elapsed < MIN_REQUEST_INTERVAL_MS {
                sleep(Duration::from_millis(MIN_REQUEST_INTERVAL_MS - elapsed)).await;
            }

            let t0 = std::time::Instant::now();
            last_request_at = t0;

            match inner.fetch_once().await {
                Ok(PollResult::Stop) => {
                    tracing::info!("SABR[{}]: stream ended (204)", inner.video_id);
                    let _ = inner.event_tx.send(SabrEvent::Finished);
                    break;
                }
                Ok(PollResult::Continue {
                    saw_media,
                    bytes,
                    elapsed_ms,
                }) => {
                    if saw_media {
                        inner.no_media_streak = 0;
                        inner.cached_ranges.clear(); // Clear cached ranges after media is seen
                        inner.update_bandwidth_estimate(bytes, elapsed_ms as u64);
                    } else if inner.next_backoff_ms > 0 {
                        inner.no_media_streak += 1;
                        inner.cached_ranges.clear(); // Clear cached ranges on backoff/stall
                    }
                }
                Err(e) => {
                    let err_str = e.to_string();
                    if err_str.contains("channel closed") {
                        tracing::debug!(
                            "SABR[{}]: audio channel closed during fetch — exiting poll loop immediately",
                            inner.video_id
                        );
                        break;
                    }

                    let is_enforcement_err = err_str.contains("media_serving_enforcement_id_error");
                    let is_malformed = err_str.contains("malformed_config");

                    if is_enforcement_err || is_malformed {
        
                        if is_enforcement_err {
                            tracing::warn!(
                                "SABR[{}]: enforcement ID error — clearing contexts and triggering recovery",
                                inner.video_id
                            );
                            inner.sabr_contexts.clear();
                            inner.active_context_types.clear();
                        } else {
                            tracing::warn!(
                                "SABR[{}]: malformed_config — triggering recovery",
                                inner.video_id
                            );
                        }

                        inner.recovery_pending = true;
                        let _ = inner.event_tx.send(SabrEvent::Stall);

                        loop {
                            sleep(Duration::from_millis(250)).await;
                            inner.check_commands();
                            if !inner.recovery_pending {
                                break; // UpdateSession arrived, resume
                            }
                            if inner.tx.is_closed() {
                                tracing::debug!(
                                    "SABR[{}]: audio channel closed during recovery wait — exiting",
                                    inner.video_id
                                );
                                return; // Track was cleared, stop the entire SABR task
                            }
                        }
                        continue;
                    }

                    inner.no_media_streak += 1;
                    tracing::warn!(
                        "SABR[{}]: fetch failed (streak={}): {}",
                        inner.video_id,
                        inner.no_media_streak,
                        e
                    );
                    if inner.no_media_streak >= MAX_NO_MEDIA_STREAK {
                        tracing::error!(
                            "SABR[{}]: too many consecutive failures, aborting",
                            inner.video_id
                        );
                        let _ = inner.event_tx.send(SabrEvent::Error(e.to_string()));
                        break;
                    }
                    // Short sleep before retry
                    sleep(Duration::from_millis(1000)).await;
                }
            }
        }

        tracing::debug!("SABR[{}]: polling loop exited", inner.video_id);
    });

    Some((rx, event_rx, cmd_tx, handle))
}

pub fn best_format_mime(config: &SabrConfig) -> Option<String> {
    config.best_audio_format().map(|f| f.mime_type.clone())
}
