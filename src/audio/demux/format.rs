//! Audio format detection via header byte sniffing.
//!
//! `detect_format` reads the first bytes of a stream and identifies the
//! container format without seeking, so the caller can pick a decode path
//! before handing the stream to a full demuxer.

/// Known container formats that the pipeline can handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioFormat {
    /// EBML / Matroska WebM container — typically holds Opus or VP9.
    WebmOpus,
    /// MPEG-4 Part 14 / MP4 / M4A / AAC container.
    Mp4,
    /// MPEG Audio Layer III.
    Mp3,
    /// OGG container (Vorbis, Opus, FLAC).
    Ogg,
    /// FLAC (native framing, not in OGG).
    Flac,
    /// WAV / RIFF.
    Wav,
    /// Format could not be identified from the header.
    Unknown,
}

impl AudioFormat {
    /// Returns the filename extension hint suitable for `symphonia::Hint`.
    pub fn as_ext(self) -> &'static str {
        match self {
            AudioFormat::WebmOpus => "webm",
            AudioFormat::Mp4 => "m4a",
            AudioFormat::Mp3 => "mp3",
            AudioFormat::Ogg => "ogg",
            AudioFormat::Flac => "flac",
            AudioFormat::Wav => "wav",
            AudioFormat::Unknown => "",
        }
    }

    /// `true` if this format carries raw Opus that can be sent to Discord
    /// without re-encoding (zero-transcode path).
    pub fn is_opus_passthrough(self) -> bool {
        matches!(self, AudioFormat::WebmOpus | AudioFormat::Ogg)
    }
}

impl std::fmt::Display for AudioFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            AudioFormat::WebmOpus => "WebM/Opus",
            AudioFormat::Mp4 => "MPEG-4",
            AudioFormat::Mp3 => "MP3",
            AudioFormat::Ogg => "OGG",
            AudioFormat::Flac => "FLAC",
            AudioFormat::Wav => "WAV",
            AudioFormat::Unknown => "Unknown",
        })
    }
}

/// Sniff the container format from the first bytes of arbitrary data.
///
/// Requires at least 4 bytes.  Returns `AudioFormat::Unknown` for anything
/// not in the table above.
pub fn detect_format(header: &[u8]) -> AudioFormat {
    if header.len() < 4 {
        return AudioFormat::Unknown;
    }

    // EBML magic (WebM / Matroska): 0x1A 45 DF A3
    if header.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        return AudioFormat::WebmOpus;
    }

    // ftyp / MP4 / M4A: bytes [4..8] == "ftyp" — but also check "moov" at 0
    if header.len() >= 8 && &header[4..8] == b"ftyp" {
        return AudioFormat::Mp4;
    }

    // OGG: "OggS"
    if header.starts_with(b"OggS") {
        return AudioFormat::Ogg;
    }

    // FLAC: "fLaC"
    if header.starts_with(b"fLaC") {
        return AudioFormat::Flac;
    }

    // WAV: "RIFF" + 4 bytes + "WAVE"
    if header.starts_with(b"RIFF") && header.len() >= 12 && &header[8..12] == b"WAVE" {
        return AudioFormat::Wav;
    }

    // MP3: ID3 tag or sync word (0xFF 0xFB / 0xFF 0xFA / 0xFF 0xF3 etc.)
    if header.starts_with(b"ID3") {
        return AudioFormat::Mp3;
    }
    if header[0] == 0xFF && (header[1] & 0xE0) == 0xE0 {
        return AudioFormat::Mp3;
    }

    AudioFormat::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_webm() {
        let hdr = [0x1A, 0x45, 0xDF, 0xA3, 0x00, 0x00, 0x00, 0x00];
        assert_eq!(detect_format(&hdr), AudioFormat::WebmOpus);
    }

    #[test]
    fn detect_mp4() {
        let hdr = b"\x00\x00\x00\x1Cftypisom";
        assert_eq!(detect_format(hdr), AudioFormat::Mp4);
    }

    #[test]
    fn detect_ogg() {
        assert_eq!(detect_format(b"OggS\x00"), AudioFormat::Ogg);
    }

    #[test]
    fn detect_unknown() {
        assert_eq!(
            detect_format(&[0x00, 0x00, 0x00, 0x00]),
            AudioFormat::Unknown
        );
    }
}
