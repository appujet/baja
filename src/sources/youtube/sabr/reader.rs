use crate::sources::youtube::sabr::parser::decoders::*;
use crate::sources::youtube::sabr::parser::{ProtoReader, UmpReader};
use crate::sources::youtube::sabr::structs::*;
use crate::sources::youtube::sabr::writer::*;
use flume::{Receiver, Sender};
use std::io::{Read, Seek, SeekFrom};
use symphonia::core::io::MediaSource;
use tracing::{debug, error};

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
        formats: Vec<FormatId>,
    ) -> (Self, tokio::task::JoinHandle<()>) {
        let (tx, rx) = flume::bounded(10);

        let handle = tokio::spawn(async move {
            if let Err(e) = run_sabr_loop(
                server_abr_url,
                ustreamer_config,
                client_name,
                client_version,
                visitor_data,
                formats,
                tx,
            )
            .await
            {
                error!("SABR loop error: {}", e);
            }
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
    #[allow(dead_code)]
    request_number: i32,
    player_time_ms: i64,
    bandwidth_estimate: i64,
    playback_cookie: Option<Vec<u8>>,
    url: reqwest::Url,
}

async fn run_sabr_loop(
    server_abr_url: String,
    ustreamer_config: Vec<u8>,
    client_name: i32,
    client_version: String,
    visitor_data: String,
    formats: Vec<FormatId>,
    tx: Sender<Vec<u8>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let client = reqwest::Client::new();
    let mut url = reqwest::Url::parse(&server_abr_url)?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("alr", "yes");
        query.append_pair("ump", "1");
        query.append_pair("srfvp", "1");
    }

    // Select a suitable audio itag (priority: 140, 251, 141)
    let audio_format = formats
        .iter()
        .find(|f| f.itag == 140)
        .or_else(|| formats.iter().find(|f| f.itag == 251))
        .or_else(|| formats.iter().find(|f| f.itag == 141))
        .cloned();

    let mut session = SabrSession {
        request_number: 0,
        player_time_ms: 0,
        bandwidth_estimate: 5_000_000,
        playback_cookie: None,
        url,
    };

    loop {
        let mut writer = ProtoWriter::new();

        let request = VideoPlaybackAbrRequest {
            client_abr_state: Some(ClientAbrState {
                bandwidth_estimate: session.bandwidth_estimate,
                player_time_ms: session.player_time_ms,
                enabled_track_types_bitfield: 1, // AUDIO_ONLY
                visibility: 1,
                playback_rate: 1.0,
                player_state: 1,
                audio_track_id: String::new(),
                ..Default::default()
            }),
            selected_format_ids: audio_format
                .as_ref()
                .map(|f| vec![f.clone()])
                .unwrap_or_default(),
            video_playback_ustreamer_config: ustreamer_config.clone(),
            preferred_audio_format_ids: audio_format
                .as_ref()
                .map(|f| vec![f.clone()])
                .unwrap_or_default(),
            streamer_context: Some(StreamerContext {
                client_info: Some(ClientInfo {
                    client_name,
                    client_version: client_version.clone(),
                }),
                playback_cookie: session.playback_cookie.clone(),
                ..Default::default()
            }),
            ..Default::default()
        };

        encode_video_playback_abr_request(&request, &mut writer);
        let body = writer.finish();

        debug!(
            "Sending SABR request (RN={}) to {}",
            session.request_number, session.url
        );

        let res = client
            .post(session.url.clone())
            .header("X-Goog-Visitor-Id", &visitor_data)
            .header("X-YouTube-Client-Name", client_name.to_string())
            .header("X-YouTube-Client-Version", &client_version)
            .body(body)
            .send()
            .await?;

        if !res.status().is_success() {
            error!("SABR request failed with status: {}", res.status());
            break;
        }

        let body_bytes = res.bytes().await?;
        let mut ump_reader = UmpReader::new(&body_bytes);

        let mut media_received_in_this_batch = false;

        while let Some(part) = ump_reader.next_part() {
            match part.part_type {
                21 => {
                    // Media
                    if part.data.len() > 1 {
                        // First byte is headerId
                        if tx.send_async(part.data[1..].to_vec()).await.is_err() {
                            return Ok(()); // Receiver dropped
                        }
                        media_received_in_this_batch = true;
                    }
                }
                20 => {
                    // MediaHeader
                    let mut reader = ProtoReader::new(&part.data);
                    let header = decode_media_header(&mut reader, part.data.len());
                    if !header.duration_ms.is_empty() {
                        if let Ok(dur) = header.duration_ms.parse::<i64>() {
                            session.player_time_ms += dur;
                        }
                    }
                }
                35 => {
                    // NextRequestPolicy
                    let mut reader = ProtoReader::new(&part.data);
                    let policy = decode_next_request_policy(&mut reader, part.data.len());
                    if let Some(cookie) = policy.playback_cookie {
                        session.playback_cookie = Some(cookie);
                    }
                    // Could handle backoff here
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
                _ => {}
            }
        }

        session.request_number += 1;

        if !media_received_in_this_batch {
            // If no media received, don't loop too fast
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        } else {
            // Small delay to avoid hammering
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }
    }

    Ok(())
}
