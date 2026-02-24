use serde_json::Value;

/// All the configuration needed to start a SABR streaming session.
/// Extracted from the YouTube player API response's `streamingData`.
#[derive(Debug, Clone)]
pub struct SabrConfig {
    /// `streamingData.serverAbrStreamingUrl`
    pub server_abr_url: String,
    /// `streamingData.videoPlaybackUstreamerConfig` (base64 blob)
    pub ustreamer_config: String,
    /// Optional visitor data from the session context
    pub visitor_data: Option<String>,
    /// Optional PoToken (base64) obtained from yt-cipher
    pub po_token: Option<String>,
    /// YouTube client name ID (1 = WEB)
    pub client_name_id: i32,
    /// YouTube client version string
    pub client_version: String,
    /// User-Agent to use for SABR HTTP requests
    pub user_agent: String,
    /// All audio formats available in the player response
    pub formats: Vec<SabrFormat>,
    /// Start position in milliseconds
    pub start_time_ms: u64,
}

/// Represents one entry from `streamingData.adaptiveFormats`.
#[derive(Debug, Clone)]
pub struct SabrFormat {
    pub itag: i32,
    pub last_modified: String,
    pub xtags: Option<String>,
    pub mime_type: String,
    pub audio_track_id: Option<String>,
    pub bitrate: u64,
    pub average_bitrate: u64,
    pub audio_channels: u32,
    pub is_default_audio_track: bool,
    pub is_drc: bool,
}

impl SabrFormat {
    pub fn is_audio(&self) -> bool {
        self.mime_type.starts_with("audio/")
    }

    pub fn effective_bitrate(&self) -> u64 {
        if self.average_bitrate > 0 {
            self.average_bitrate
        } else {
            self.bitrate
        }
    }

    pub fn format_info(&self) -> FormatInfo {
        if self.mime_type.contains("audio/webm") {
            FormatInfo::Webm
        } else if self.mime_type.contains("audio/mp4") {
            FormatInfo::Mp4
        } else {
            FormatInfo::Unknown
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum FormatInfo {
    Webm = 0, 
    Mp4 = 1,
    Unknown = 2,
}

impl SabrConfig {

    pub fn from_player_response(
        response: &Value,
        visitor_data: Option<String>,
        po_token: Option<String>,
        client_name_id: i32,
        client_version: String,
        user_agent: String,
    ) -> Option<Self> {
        let streaming_data = match response.get("streamingData") {
            Some(sd) => sd,
            None => {
                tracing::warn!("SABR config: missing 'streamingData' in player response");
                return None;
            }
        };

        let server_abr_url = match streaming_data
            .get("serverAbrStreamingUrl")
            .and_then(|v| v.as_str())
            .map(str::to_string)
        {
            Some(u) => u,
            None => {
                tracing::debug!("SABR config: missing 'serverAbrStreamingUrl'");
                return None;
            }
        };

        // reads ustreamer_config from:
        // playerConfig.mediaCommonConfig.mediaUstreamerRequestConfig.videoPlaybackUstreamerConfig
        // NOT from streamingData.videoPlaybackUstreamerConfig
        let ustreamer_config = response
            .get("playerConfig")
            .and_then(|v| v.get("mediaCommonConfig"))
            .and_then(|v| v.get("mediaUstreamerRequestConfig"))
            .and_then(|v| v.get("videoPlaybackUstreamerConfig"))
            .and_then(|v| v.as_str())
            .map(str::to_string)
            // Fallback: some responses put it directly in streamingData
            .or_else(|| {
                streaming_data
                    .get("videoPlaybackUstreamerConfig")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            });

        let ustreamer_config = match ustreamer_config {
            Some(c) => c,
            None => {
                tracing::warn!(
                    "SABR config: missing 'videoPlaybackUstreamerConfig' in both \
                     playerConfig.mediaCommonConfig.mediaUstreamerRequestConfig and \
                     streamingData — this video may not support SABR"
                );
                return None;
            }
        };


        let adaptive_formats = streaming_data
            .get("adaptiveFormats")
            .and_then(|v| v.as_array());

        let mut formats = Vec::new();

        if let Some(af) = adaptive_formats {
            for fmt in af {
                let itag = fmt.get("itag").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                if itag == 0 {
                    continue;
                }

                let mime_type = fmt
                    .get("mimeType")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                // Only audio formats for SABR audio streaming
                if !mime_type.starts_with("audio/") {
                    continue;
                }

                let last_modified = fmt
                    .get("lastModified")
                    .and_then(|v| v.as_str())
                    .unwrap_or("0")
                    .to_string();

                let xtags = fmt
                    .get("xtags")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);

                let audio_track_id = fmt
                    .get("audioTrackId")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);

                let bitrate = fmt
                    .get("bitrate")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);

                let average_bitrate = fmt
                    .get("averageBitrate")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);

                let audio_channels = fmt
                    .get("audioChannels")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(2) as u32;

                let is_default_audio_track = fmt
                    .get("audioQuality")
                    .and_then(|v| v.as_str())
                    .map(|q| q == "AUDIO_QUALITY_MEDIUM" || q == "AUDIO_QUALITY_HIGH")
                    .unwrap_or(false);

                let is_drc = fmt
                    .get("isDrc")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);

                formats.push(SabrFormat {
                    itag,
                    last_modified,
                    xtags,
                    mime_type,
                    audio_track_id,
                    bitrate,
                    average_bitrate,
                    audio_channels,
                    is_default_audio_track,
                    is_drc,
                });
            }
        }

        if formats.is_empty() {
            tracing::warn!(
                "SABR config: no audio formats found in adaptiveFormats"
            );
        } else {
            tracing::debug!(
                "SABR config: {} audio formats parsed (best itag={})",
                formats.len(),
                formats
                    .iter()
                    .max_by_key(|f| f.effective_bitrate())
                    .map(|f| f.itag)
                    .unwrap_or(0)
            );
        }

        Some(SabrConfig {
            server_abr_url,
            ustreamer_config,
            visitor_data,
            po_token,
            client_name_id,
            client_version,
            user_agent,
            formats,
            start_time_ms: 0,
        })
    }


    /// Select the best audio format for SABR streaming.
    /// Prefers `audio/webm` (opus) first, then `audio/mp4` (AAC).
    /// Higher bitrate wins within the same format type; prefers non-DRC and default audio tracks.
    pub fn best_audio_format(&self) -> Option<&SabrFormat> {
        let mut best: Option<&SabrFormat> = None;

        // First pass: default audio track only
        for fmt in &self.formats {
            if !fmt.is_audio() || !fmt.is_default_audio_track {
                continue;
            }
            if is_better_format(fmt, best) {
                best = Some(fmt);
            }
        }

        if best.is_some() {
            return best;
        }

        // Second pass: any audio format
        for fmt in &self.formats {
            if !fmt.is_audio() {
                continue;
            }
            if is_better_format(fmt, best) {
                best = Some(fmt);
            }
        }

        best
    }
}

fn is_better_format<'a>(fmt: &'a SabrFormat, current_best: Option<&'a SabrFormat>) -> bool {
    let Some(best) = current_best else {
        return true;
    };

    // Mp4/AAC preferred over Webm/Opus
    let fi = fmt.format_info();
    let bi = best.format_info();

    if fi != bi {
        return fi < bi;
    }

    // Prefer non-DRC
    if fmt.is_drc && !best.is_drc {
        return false;
    }
    if !fmt.is_drc && best.is_drc {
        return true;
    }

    // Higher effective bitrate wins
    fmt.effective_bitrate() > best.effective_bitrate()
}
