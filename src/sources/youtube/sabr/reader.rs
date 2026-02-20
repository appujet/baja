use std::io::{Read, Seek, SeekFrom};

use base64::Engine;
use flume::{Receiver, Sender};
use symphonia::core::io::MediaSource;
use tracing::{debug, error, info, warn};

use crate::sources::youtube::sabr::{
    parser::{ProtoReader, UmpReader, decoders::*},
    structs::*,
    writer::*,
};

pub struct SabrReader {
    rx: Receiver<Vec<u8>>,
    current_chunk: Vec<u8>,
    current_pos: usize,
    #[allow(dead_code)]
    total_pos: u64,
}

impl SabrReader {
    pub fn new(
        server_abr_url: String,
        ustreamer_config: Vec<u8>,
        client_name: i32,
        client_version: String,
        visitor_data: String,
        video_id: String,
        formats: Vec<FormatId>,
    ) -> (Self, std::thread::JoinHandle<()>) {
        let (tx, rx) = flume::bounded(10);

        let handle = std::thread::spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    error!("Failed to build SabrReader tokio runtime: {}", e);
                    return;
                }
            };

            rt.block_on(async move {
                let mut session = SabrSession {
                    request_number: 0,
                    player_time_ms: 0,
                    bandwidth_estimate: 5_000_000,
                    playback_cookie: None,
                    po_token: None,
                    sabr_contexts: std::collections::HashMap::new(),
                    active_sabr_context_types: std::collections::HashSet::new(),
                    pending_ranges_headers: std::collections::HashMap::new(),
                    cached_buffered_ranges: None,
                    url: reqwest::Url::parse(&server_abr_url)
                        .unwrap_or_else(|_| reqwest::Url::parse("https://youtube.com").unwrap()),
                    ustreamer_config,
                    visitor_data,
                };
                let mut initialized_formats = std::collections::HashSet::new();
                let mut last_forwarded_sequence = std::collections::HashMap::new();

                loop {
                    if session.po_token.is_none() {
                        match crate::sources::youtube::sabr::potoken::PoTokenManager::generate_botguard_token(&video_id, Some(&session.visitor_data)).await {
                            Ok((po_token_b64, new_vd)) => {
                                if let Ok(decoded) = base64::prelude::BASE64_URL_SAFE_NO_PAD.decode(&po_token_b64).or_else(|_| base64::prelude::BASE64_URL_SAFE.decode(&po_token_b64)) {
                                    session.po_token = Some(decoded);
                                    if !new_vd.is_empty() {
                                        session.visitor_data = new_vd;
                                    }
                                    tracing::info!("BotGuard PoToken generated via rustypipe-botguard (len={})", po_token_b64.len());
                                }
                            }
                            Err(e) => tracing::warn!("Failed to generate BotGuard token: {}", e),
                        }
                    }

                    if let Err(e) = run_sabr_loop(
                        &mut session,
                        client_name,
                        &client_version,
                        &video_id,
                        &formats,
                        &tx,
                        &mut initialized_formats,
                        &mut last_forwarded_sequence,
                    )
                    .await
                    {
                        if e.to_string() == "sab_stall" {
                            tracing::warn!("SABR stream stalled. Attempting one-time recovery...");
                            match refresh_sabr_session(&video_id, &session.visitor_data).await {
                                Ok((new_url, new_config, new_vd)) => {
                                    if let Ok(url) = reqwest::Url::parse(&new_url) {
                                        session.url = url;
                                    }
                                    session.ustreamer_config = new_config;
                                    session.visitor_data = new_vd;
                                    session.po_token = None; // Force re-fetch of po_token
                                    // Notice we DO NOT RESET `player_time_ms` or `request_number`!
                                    // The deduplicator sets are also NOT reset!
                                    tracing::info!(
                                        "Recovery successful. Restarting SABR loop from {}ms",
                                        session.player_time_ms
                                    );
                                    continue;
                                }
                                Err(e) => {
                                    error!("SABR recovery failed: {}", e);
                                    break;
                                }
                            }
                        } else {
                            error!("SABR loop error: {}", e);
                            break;
                        }
                    } else {
                        break;
                    }
                }
            });
        });

        (
            Self {
                rx,
                current_chunk: Vec::new(),
                current_pos: 0,
                total_pos: 0,
            },
            handle,
        )
    }
}

impl Read for SabrReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.current_pos >= self.current_chunk.len() {
            match self.rx.recv() {
                Ok(chunk) => {
                    self.current_chunk = chunk;
                    self.current_pos = 0;
                }
                Err(_) => return Ok(0), // EOF
            }
        }

        let n = std::cmp::min(buf.len(), self.current_chunk.len() - self.current_pos);
        buf[..n].copy_from_slice(&self.current_chunk[self.current_pos..self.current_pos + n]);
        self.current_pos += n;
        self.total_pos += n as u64;
        Ok(n)
    }
}

impl Seek for SabrReader {
    fn seek(&mut self, _pos: SeekFrom) -> std::io::Result<u64> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Seeking not supported in SabrReader",
        ))
    }
}

impl MediaSource for SabrReader {
    fn is_seekable(&self) -> bool {
        false
    }
    fn byte_len(&self) -> Option<u64> {
        None
    }
}

struct SabrSession {
    request_number: i32,
    player_time_ms: i64,
    bandwidth_estimate: i64,
    playback_cookie: Option<Vec<u8>>,
    po_token: Option<Vec<u8>>,
    sabr_contexts: std::collections::HashMap<i32, SabrContextUpdate>,
    active_sabr_context_types: std::collections::HashSet<i32>,
    pending_ranges_headers: std::collections::HashMap<i32, Vec<MediaHeader>>,
    cached_buffered_ranges: Option<Vec<BufferedRange>>,
    url: reqwest::Url,
    ustreamer_config: Vec<u8>,
    visitor_data: String,
}

/// Re-fetch a fresh `serverAbrStreamingUrl` + `ustreamerConfig` from Android /player API.
async fn refresh_sabr_session(
    video_id: &str,
    visitor_data: &str,
) -> Result<(String, Vec<u8>, String), Box<dyn std::error::Error + Send + Sync>> {
    debug!("SABR recovery: Calling Android /player API for fresh SABR URL...");

    let http = reqwest::Client::builder()
        .user_agent("com.google.android.youtube/20.01.35 (Linux; U; Android 14) identity")
        .timeout(std::time::Duration::from_secs(10))
        .build()?;

    let mut client_context = serde_json::json!({
        "clientName": "ANDROID",
        "clientVersion": "20.01.35",
        "userAgent": "com.google.android.youtube/20.01.35 (Linux; U; Android 14) identity",
        "deviceMake": "Google",
        "deviceModel": "Pixel 6",
        "osName": "Android",
        "osVersion": "14",
        "androidSdkVersion": "34",
        "hl": "en",
        "gl": "US"
    });

    if !visitor_data.is_empty() {
        if let Some(obj) = client_context.as_object_mut() {
            obj.insert("visitorData".to_string(), visitor_data.into());
        }
    }

    let body = serde_json::json!({
        "context": {
            "client": client_context,
            "user": { "lockedSafetyMode": false },
            "request": { "useSsl": true }
        },
        "videoId": video_id,
        "contentCheckOk": true,
        "racyCheckOk": true
    });

    let url = format!(
        "{}/youtubei/v1/player?prettyPrint=false",
        crate::sources::youtube::clients::common::INNERTUBE_API
    );

    let res = http
        .post(&url)
        .header("X-YouTube-Client-Name", "3")
        .header("X-YouTube-Client-Version", "20.01.35")
        .header("X-Goog-Visitor-Id", visitor_data)
        .json(&body)
        .send()
        .await?;

    if !res.status().is_success() {
        return Err("Android player request failed during recovery".into());
    }

    let response: serde_json::Value = res.json().await?;

    let streaming_data = response
        .get("streamingData")
        .ok_or("No streamingData in recovery response")?;

    let new_url = streaming_data
        .get("serverAbrStreamingUrl")
        .and_then(|v| v.as_str())
        .ok_or("No serverAbrStreamingUrl in recovery response")?
        .to_string();

    let new_ustreamer = response
        .get("playerConfig")
        .and_then(|p| p.get("mediaCommonConfig"))
        .and_then(|m| m.get("mediaUstreamerRequestConfig"))
        .and_then(|m| m.get("videoPlaybackUstreamerConfig"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let new_ustreamer_bytes = base64::prelude::BASE64_STANDARD
        .decode(new_ustreamer)
        .or_else(|_| base64::prelude::BASE64_URL_SAFE.decode(new_ustreamer))
        .unwrap_or_default();

    let new_visitor_data = response
        .get("responseContext")
        .and_then(|r| r.get("visitorData"))
        .and_then(|v| v.as_str())
        .unwrap_or(visitor_data)
        .to_string();

    debug!(
        "SABR recovery: Got fresh URL ({} bytes) and visitor data.",
        new_url.len()
    );
    Ok((new_url, new_ustreamer_bytes, new_visitor_data))
}

async fn run_sabr_loop(
    session: &mut SabrSession,
    client_name: i32,
    client_version: &str,
    video_id: &str,
    formats: &[FormatId],
    tx: &Sender<Vec<u8>>,
    initialized_formats: &mut std::collections::HashSet<i32>,
    last_forwarded_sequence: &mut std::collections::HashMap<i32, i32>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let client = reqwest::Client::new();
    {
        let mut query = session.url.query_pairs_mut();
        query.append_pair("alr", "yes");
        query.append_pair("ump", "1");
        query.append_pair("srfvp", "1");
    }

    let audio_format = formats
        .iter()
        .find(|f| f.itag == 140)
        .or_else(|| formats.iter().find(|f| f.itag == 251))
        .or_else(|| formats.iter().find(|f| f.itag == 141))
        .cloned();

    let mut last_recovery_at = std::time::Instant::now() - std::time::Duration::from_secs(10);
    let mut recovery_attempted = false;
    // Maps header_id -> (duration_ms, is_skipped).
    // Populated on MediaHeader, consumed on MediaEnd to advance player_time_ms.
    let mut pending_header_info: std::collections::HashMap<i32, (i64, bool)> =
        std::collections::HashMap::new();

    loop {
        // Add rn parameter to URL
        let mut req_url = session.url.clone();
        req_url
            .query_pairs_mut()
            .append_pair("rn", &session.request_number.to_string());

        let request = VideoPlaybackAbrRequest {
            client_abr_state: Some(ClientAbrState {
                player_time_ms: session.player_time_ms,
                bandwidth_estimate: session.bandwidth_estimate,
                enabled_track_types_bitfield: 1,
                player_state: 1,
                visibility: 1,
                playback_rate: 1.0,
                last_manual_selected_resolution: 1080,
                sticky_resolution: 1080,
                client_viewport_is_flexible: false,
                time_since_last_action_ms: 0,
                drc_enabled: false,
                audio_track_id: String::new(),
            }),
            selected_format_ids: if session.request_number > 0 {
                audio_format.iter().cloned().collect()
            } else {
                vec![]
            },
            buffered_ranges: {
                if session.cached_buffered_ranges.is_none() {
                    let mut ranges = Vec::new();
                    for (itag, headers) in session.pending_ranges_headers.drain() {
                        if headers.is_empty() {
                            continue;
                        }
                        let start_h = &headers[0];
                        let end_h = &headers[headers.len() - 1];
                        let duration_total: i64 = headers
                            .iter()
                            .filter_map(|h| h.duration_ms.parse::<i64>().ok())
                            .sum();

                        let format_id = start_h.format_id.clone().unwrap_or(FormatId {
                            itag,
                            last_modified: None,
                            xtags: start_h.xtags.clone(),
                        });

                        // Construct time_range mimicking NodeLink
                        let timescale = start_h
                            .time_range
                            .as_ref()
                            .map(|tr| tr.timescale)
                            .unwrap_or(1000);
                        let start_ms = start_h.start_ms.parse::<i64>().unwrap_or(0);
                        let duration_ticks = (duration_total * (timescale as i64)) / 1000;

                        let time_range = Some(TimeRange {
                            start_ticks: start_ms,
                            duration_ticks,
                            timescale,
                        });

                        ranges.push(BufferedRange {
                            format_id: Some(format_id),
                            start_time_ms: start_ms,
                            duration_ms: duration_total,
                            start_segment_index: start_h.sequence_number,
                            end_segment_index: end_h.sequence_number,
                            time_range,
                        });
                    }
                    session.cached_buffered_ranges = Some(ranges);
                }
                session.cached_buffered_ranges.as_ref().unwrap().clone()
            },
            player_time_ms: session.player_time_ms,
            video_playback_ustreamer_config: session.ustreamer_config.clone(),
            preferred_audio_format_ids: audio_format.iter().cloned().collect(),
            preferred_video_format_ids: vec![],
            streamer_context: Some(StreamerContext {
                client_info: Some(ClientInfo {
                    client_name,
                    client_version: client_version.to_string(),
                }),
                po_token: session.po_token.clone(),
                playback_cookie: session.playback_cookie.clone(),
                sabr_contexts: session
                    .active_sabr_context_types
                    .iter()
                    .filter_map(|t| session.sabr_contexts.get(t))
                    .map(|ctx| SabrContext {
                        context_type: ctx.context_type,
                        value: ctx.value.clone(),
                    })
                    .collect(),
                unsent_sabr_contexts: session
                    .sabr_contexts
                    .keys()
                    .filter(|k| !session.active_sabr_context_types.contains(k))
                    .copied()
                    .collect(),
            }),
        };

        let mut writer = ProtoWriter::new();
        encode_video_playback_abr_request(&request, &mut writer);
        let body = writer.finish();

        debug!(
            "Sending SABR request (RN={}) to {}",
            session.request_number,
            req_url.as_str()
        );

        let mut res = client
            .post(req_url.as_str())
            .header("Content-Type", "application/x-protobuf")
            .header("Accept", "application/vnd.yt-ump")
            .header("Origin", "https://www.youtube.com")
            .header("Referer", "https://www.youtube.com/")
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/143.0.0.0 Safari/537.36")
            .header("X-Goog-Visitor-Id", &session.visitor_data)
            .header("X-YouTube-Client-Name", client_name.to_string())
            .header("X-YouTube-Client-Version", client_version)
            .body(body)
            .send()
            .await?;

        if !res.status().is_success() {
            error!("SABR request failed: {}", res.status());
            break;
        }

        // Key diagnostic: what are we telling YouTube our player position is?
        debug!(
            "SABR RN={} → playerTimeMs={} poToken={} bufferedRanges={}",
            session.request_number,
            session.player_time_ms,
            if session.po_token.is_some() {
                "yes"
            } else {
                "NO"
            },
            session
                .cached_buffered_ranges
                .as_ref()
                .map(|r| r.len())
                .unwrap_or(0)
        );

        let mut ump_reader = UmpReader::new();
        let mut media_received_in_this_batch = false;
        let mut parts_received = 0;
        let mut stall_detected = false;
        let mut backoff_time_ms = 0;
        // Reset per-request dedup state. `skipped_header_ids` must be fresh for each
        // SABR request-response cycle; otherwise header IDs from previous responses
        // (which start back at 0) suppress real new media data and cause the loop.
        // `initialized_formats` and `last_forwarded_sequence` intentionally persist.
        let mut skipped_header_ids: std::collections::HashSet<i32> =
            std::collections::HashSet::new();
        pending_header_info.clear();
        let mut last_protection_status: Option<i32> = None;
        let mut segments_forwarded: Vec<(i32, i32)> = Vec::new(); // (itag, seq)

        loop {
            match res.chunk().await {
                Ok(Some(chunk)) => {
                    ump_reader.append(&chunk);

                    while let Some(part) = ump_reader.next_part() {
                        parts_received += 1;
                        match part.part_type {
                            21 => {
                                // Media data — first byte is headerId
                                if part.data.len() > 1 {
                                    let header_id = part.data[0] as i32;

                                    if skipped_header_ids.contains(&header_id) {
                                        continue;
                                    }

                                    if tx.send_async(part.data[1..].to_vec()).await.is_err() {
                                        return Ok(());
                                    }
                                    media_received_in_this_batch = true;
                                }
                            }
                            20 => {
                                // MediaHeader — decode and track for dedup
                                let mut reader = ProtoReader::new(&part.data);
                                let header = decode_media_header(&mut reader, part.data.len());

                                if header.is_init_seg {
                                    // Init segment dedup: only forward the FIRST init per itag
                                    let fmt_key = header.itag;
                                    if initialized_formats.contains(&fmt_key) {
                                        skipped_header_ids.insert(header.header_id);
                                    } else {
                                        debug!(
                                            "First init segment: itag={} header_id={}",
                                            fmt_key, header.header_id
                                        );
                                        initialized_formats.insert(fmt_key);
                                        skipped_header_ids.remove(&header.header_id);
                                    }
                                } else {
                                    // Data segment dedup: skip if sequence_number <= last forwarded
                                    let seq = header.sequence_number;
                                    let itag = header.itag;
                                    let last_seq =
                                        last_forwarded_sequence.get(&itag).copied().unwrap_or(-1);

                                    if seq > 0 && seq <= last_seq {
                                        debug!(
                                            "Skipping duplicate: itag={} seq={} (last_forwarded={})",
                                            itag, seq, last_seq
                                        );
                                        skipped_header_ids.insert(header.header_id);
                                    } else {
                                        // New segment — log it
                                        debug!(
                                            "Forwarding new segment: itag={} seq={} dur={}ms header_id={}",
                                            itag, seq, header.duration_ms, header.header_id
                                        );
                                        if seq > 0 {
                                            last_forwarded_sequence.insert(itag, seq);
                                            segments_forwarded.push((itag, seq));
                                        }
                                        skipped_header_ids.remove(&header.header_id);
                                    }
                                }

                                // Record duration so we can advance player_time_ms
                                // in MediaEnd (after data is fully received), matching
                                // NodeLink's totalDownloadedMs += segmentDuration in handleMediaEnd.
                                let is_skipped = skipped_header_ids.contains(&header.header_id);
                                let duration_ms = header.duration_ms.parse::<i64>().unwrap_or(0);
                                pending_header_info
                                    .insert(header.header_id, (duration_ms, is_skipped));

                                // Save header to acknowledge it in the next bufferedRanges SABR payload
                                if !header.is_init_seg && !is_skipped {
                                    session
                                        .pending_ranges_headers
                                        .entry(header.itag)
                                        .or_default()
                                        .push(header);
                                }
                            }
                            35 => {
                                // NextRequestPolicy
                                let mut reader = ProtoReader::new(&part.data);
                                let policy =
                                    decode_next_request_policy(&mut reader, part.data.len());
                                backoff_time_ms = policy.backoff_time_ms;
                                if let Some(cookie) = policy.playback_cookie {
                                    session.playback_cookie = Some(cookie);
                                }
                            }
                            43 => {
                                // SabrRedirect
                                let mut reader = ProtoReader::new(&part.data);
                                let redirect = decode_sabr_redirect(&mut reader, part.data.len());
                                if let Ok(new_url) = reqwest::Url::parse(&redirect.url) {
                                    session.url = new_url;
                                    {
                                        let mut query = session.url.query_pairs_mut();
                                        query.append_pair("alr", "yes");
                                        query.append_pair("ump", "1");
                                        query.append_pair("srfvp", "1");
                                    }
                                }
                            }
                            44 => {
                                // SabrError
                                let mut reader = ProtoReader::new(&part.data);
                                let err = decode_sabr_error(&mut reader, part.data.len());
                                error!("SABR Error: {} (Code: {})", err.error_type, err.code);
                            }
                            57 => {
                                // SabrContextUpdate
                                let mut reader = ProtoReader::new(&part.data);
                                let ctx = decode_sabr_context_update(&mut reader, part.data.len());
                                if ctx.context_type != 0 && !ctx.value.is_empty() {
                                    let ctype = ctx.context_type;
                                    let send_by_default = ctx.send_by_default;
                                    session.sabr_contexts.insert(ctype, ctx);
                                    if send_by_default {
                                        session.active_sabr_context_types.insert(ctype);
                                    }
                                }
                            }
                            58 => {
                                // StreamProtectionStatus
                                let mut reader = ProtoReader::new(&part.data);
                                let status =
                                    decode_stream_protection_status(&mut reader, part.data.len());
                                let new_status = status.status;
                                // Only log when status changes to avoid repeat spam
                                if last_protection_status != Some(new_status) {
                                    match new_status {
                                        1 => {
                                            info!("SABR: StreamProtectionStatus 1 (OK — attested)")
                                        }
                                        2 => warn!(
                                            "SABR: StreamProtectionStatus 2 (Limited Playback — PO token required or invalid)"
                                        ),
                                        3 => info!(
                                            "SABR: StreamProtectionStatus 3 (Attestation pending)"
                                        ),
                                        s => warn!("SABR: StreamProtectionStatus {}", s),
                                    }
                                    last_protection_status = Some(new_status);
                                }
                                if new_status == 2 {
                                    stall_detected = true;
                                }
                            }
                            59 => {
                                // SabrContextSendingPolicy
                                let mut reader = ProtoReader::new(&part.data);
                                let policy = decode_sabr_context_sending_policy(
                                    &mut reader,
                                    part.data.len(),
                                );
                                for t in policy.start_policy {
                                    session.active_sabr_context_types.insert(t);
                                }
                                for t in policy.stop_policy {
                                    session.active_sabr_context_types.remove(&t);
                                }
                                for t in policy.discard_policy {
                                    session.sabr_contexts.remove(&t);
                                }
                            }
                            22 => {
                                // MediaEnd — first byte is header_id.
                                // This is the signal that the complete segment has been sent.
                                // Advance player_time_ms here (not at MediaHeader) to match
                                // NodeLink's totalDownloadedMs += segmentDuration in handleMediaEnd.
                                if !part.data.is_empty() {
                                    let header_id = part.data[0] as i32;
                                    if let Some((dur_ms, is_skipped)) =
                                        pending_header_info.remove(&header_id)
                                    {
                                        if !is_skipped && dur_ms > 0 {
                                            session.player_time_ms += dur_ms;
                                            debug!(
                                                "MediaEnd: header_id={} dur={}ms player_time_ms={}",
                                                header_id, dur_ms, session.player_time_ms
                                            );
                                        }
                                    }
                                }
                            }
                            _ => {
                                // Intentionally silent for unknown types to reduce log noise
                            }
                        }
                    }
                }

                Ok(None) => {
                    break; // EOF
                }
                Err(e) => {
                    error!("SABR chunk read error: {}", e);
                    break;
                }
            } // match res.chunk().await
        } // inner loop

        // Batch summary — key diagnostic
        debug!(
            "SABR batch RN={}: parts={} media={} stall={} protection={:?} forwarded={:?} playerTimeMs={}",
            session.request_number - 1,
            parts_received,
            media_received_in_this_batch,
            stall_detected,
            last_protection_status,
            segments_forwarded,
            session.player_time_ms
        );

        if media_received_in_this_batch {
            session.cached_buffered_ranges = None;
        } else if backoff_time_ms > 0 {
            session.cached_buffered_ranges = None;
        }

        if backoff_time_ms > 0 {
            tokio::time::sleep(tokio::time::Duration::from_millis(backoff_time_ms as u64)).await;
        }

        // ===== Stall recovery =====
        // Try recovery ONCE on first stall. If recovery also returns status 2
        // (PoToken still invalid), don't keep recovering — just increment RN
        // and let the stream play what YouTube gives us.
        if stall_detected && !recovery_attempted {
            let now = std::time::Instant::now();
            if now.duration_since(last_recovery_at) < std::time::Duration::from_secs(2) {
                debug!("SABR stall throttled (< 2s since last recovery). Skipping.");
                session.request_number += 1;
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                continue;
            }
            last_recovery_at = now;
            recovery_attempted = true;

            tracing::warn!("SABR stall: Attempting one-time recovery (fresh URL + PoToken)...");

            // Step 1: getTrackUrl() - call Android /player for fresh SABR URL
            match refresh_sabr_session(&video_id, &session.visitor_data).await {
                Ok((new_url, new_ustreamer, new_visitor_data)) => {
                    tracing::info!("SABR recovery: Got fresh SABR URL from Android client.");

                    // Step 2: clearBuffers() - clear all internal state
                    session.sabr_contexts.clear();
                    session.active_sabr_context_types.clear();
                    session.playback_cookie = None;
                    // Also clear initialized formats and sequence tracking so new session gets fresh init
                    initialized_formats.clear();
                    last_forwarded_sequence.clear();

                    // Step 3: updateSession() - set new URL, config, reset RN=0
                    let mut parsed_url = reqwest::Url::parse(&new_url)?;
                    {
                        let mut query = parsed_url.query_pairs_mut();
                        query.append_pair("alr", "yes");
                        query.append_pair("ump", "1");
                        query.append_pair("srfvp", "1");
                    }
                    session.url = parsed_url;
                    session.ustreamer_config = new_ustreamer;
                    session.visitor_data = new_visitor_data;
                    session.request_number = 0;

                    // Step 4: Generate fresh PoToken
                    match crate::sources::youtube::sabr::potoken::PoTokenManager::generate_botguard_token(&video_id, Some(&session.visitor_data)).await {
                        Ok((po_token_b64, new_vd)) => {
                            if let Ok(decoded) = base64::prelude::BASE64_URL_SAFE_NO_PAD.decode(&po_token_b64)
                                .or_else(|_| base64::prelude::BASE64_URL_SAFE.decode(&po_token_b64)) {
                                session.po_token = Some(decoded);
                                if !new_vd.is_empty() {
                                    session.visitor_data = new_vd;
                                }
                                tracing::info!("SABR recovery: New PoToken generated via rustypipe-botguard.");
                            }
                        }
                        Err(e) => {
                            tracing::warn!("SABR recovery: PoToken generation failed: {}", e);
                        }
                    }

                    let url_preview = &session.url.as_str()[..50.min(session.url.as_str().len())];
                    tracing::info!(
                        "SABR recovery: Session updated. RN=0, URL={}...",
                        url_preview
                    );

                    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                    continue; // Restart with RN=0 and fresh URL
                }
                Err(e) => {
                    tracing::warn!("SABR recovery failed: {}. Continuing with next RN.", e);
                    // Fall through to normal increment
                }
            }
        } else if stall_detected {
            // Recovery already attempted once; keep going but log periodically
            debug!(
                "SABR stall persists (RN={}) — waiting for recovery or PO token refresh",
                session.request_number + 1
            );
        }

        session.request_number += 1;

        if !media_received_in_this_batch {
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        } else {
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }
    }

    Ok(())
}
