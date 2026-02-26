//! Audio format detection via header byte sniffing.

use crate::common::types::AudioFormat;

/// Sniff the container format from the first bytes of arbitrary data.
///
/// Requires at least 4 bytes. Returns `AudioFormat::Unknown` for anything
/// not in the table.
pub fn detect_format(header: &[u8]) -> AudioFormat {
    if header.len() < 4 {
        return AudioFormat::Unknown;
    }

    // EBML magic (WebM / Matroska): 0x1A 45 DF A3
    if header.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        return AudioFormat::Webm;
    }

    // ftyp / MP4 / M4A: bytes [4..8] == "ftyp"
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

    // MP3: ID3 tag or sync word
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
        assert_eq!(detect_format(&hdr), AudioFormat::Webm);
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
