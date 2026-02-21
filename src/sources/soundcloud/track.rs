use std::net::IpAddr;

use flume::{Receiver, Sender};
use tracing::error;

use crate::{
  audio::processor::{AudioProcessor, DecoderCommand},
  configs::HttpProxyConfig,
  sources::plugin::PlayableTrack,
};

/// What kind of SoundCloud stream this track uses.
#[derive(Debug, Clone)]
pub enum SoundCloudStreamKind {
  /// Direct progressive MP3 stream (single HTTP URL)
  ProgressiveMp3,
  /// Direct progressive AAC stream (single HTTP URL)
  ProgressiveAac,
  /// HLS playlist with Opus/OGG segments
  HlsOpus,
  /// HLS playlist with MP3 segments
  HlsMp3,
  /// HLS playlist with AAC/TS segments
  HlsAac,
}

pub struct SoundCloudTrack {
  /// The resolved stream URL.
  /// - Progressive: direct audio URL (MP3 or AAC)
  /// - HLS: M3U8 manifest URL
  pub stream_url: String,
  pub kind: SoundCloudStreamKind,
  pub local_addr: Option<IpAddr>,
  pub proxy: Option<HttpProxyConfig>,
}

impl PlayableTrack for SoundCloudTrack {
  fn start_decoding(&self) -> (Receiver<i16>, Sender<DecoderCommand>) {
    let (tx, rx) = flume::bounded::<i16>(4096 * 4);
    let (cmd_tx, cmd_rx) = flume::unbounded::<DecoderCommand>();

    let stream_url = self.stream_url.clone();
    let kind = self.kind.clone();
    let local_addr = self.local_addr;
    let proxy = self.proxy.clone();

    std::thread::spawn(move || {
      match kind {
        // ── Progressive MP3 ────────────────────────────────────────
        SoundCloudStreamKind::ProgressiveMp3 => {
          let reader = match super::reader::SoundCloudReader::new(&stream_url, local_addr, proxy) {
            Ok(r) => Box::new(r) as Box<dyn symphonia::core::io::MediaSource>,
            Err(e) => {
              error!("SoundCloud progressive MP3: failed to open stream: {}", e);
              return;
            }
          };
          run_processor(reader, Some("mp3"), tx, cmd_rx);
        }

        // ── Progressive AAC ────────────────────────────────────────
        SoundCloudStreamKind::ProgressiveAac => {
          let reader = match super::reader::SoundCloudReader::new(&stream_url, local_addr, proxy) {
            Ok(r) => Box::new(r) as Box<dyn symphonia::core::io::MediaSource>,
            Err(e) => {
              error!("SoundCloud progressive AAC: failed to open stream: {}", e);
              return;
            }
          };
          run_processor(reader, Some("m4a"), tx, cmd_rx);
        }

        // ── HLS Opus (OGG container) ───────────────────────────────
        SoundCloudStreamKind::HlsOpus => {
          // Use HlsReader: double-buffered prefetch, segment-level
          // seeking, no TS demux needed (OGG is raw in segments).
          let reader = match crate::sources::youtube::hls::HlsReader::new(
            &stream_url,
            local_addr,
            None, // no YouTube cipher
            None, // no player URL
            proxy,
          ) {
            Ok(r) => Box::new(r) as Box<dyn symphonia::core::io::MediaSource>,
            Err(e) => {
              error!("SoundCloud HLS Opus: failed to init HlsReader: {}", e);
              return;
            }
          };
          run_processor(reader, Some("ogg"), tx, cmd_rx);
        }

        // ── HLS MP3 ────────────────────────────────────────────────
        SoundCloudStreamKind::HlsMp3 => {
          let reader = match crate::sources::youtube::hls::HlsReader::new(
            &stream_url,
            local_addr,
            None,
            None,
            proxy,
          ) {
            Ok(r) => Box::new(r) as Box<dyn symphonia::core::io::MediaSource>,
            Err(e) => {
              error!("SoundCloud HLS MP3: failed to init HlsReader: {}", e);
              return;
            }
          };
          run_processor(reader, Some("mp3"), tx, cmd_rx);
        }

        // ── HLS AAC / TS ───────────────────────────────────────────
        SoundCloudStreamKind::HlsAac => {
          // HlsReader automatically demuxes TS → ADTS if needed.
          let reader = match crate::sources::youtube::hls::HlsReader::new(
            &stream_url,
            local_addr,
            None,
            None,
            proxy,
          ) {
            Ok(r) => Box::new(r) as Box<dyn symphonia::core::io::MediaSource>,
            Err(e) => {
              error!("SoundCloud HLS AAC: failed to init HlsReader: {}", e);
              return;
            }
          };
          // Hint as "aac" so symphonia knows what to expect from ADTS stream.
          run_processor(reader, Some("aac"), tx, cmd_rx);
        }
      }
    });

    (rx, cmd_tx)
  }
}

fn run_processor(
  reader: Box<dyn symphonia::core::io::MediaSource>,
  ext_hint: Option<&str>,
  tx: flume::Sender<i16>,
  cmd_rx: flume::Receiver<DecoderCommand>,
) {
  match AudioProcessor::new(reader, ext_hint, tx, cmd_rx) {
    Ok(mut p) => {
      if let Err(e) = p.run() {
        error!("SoundCloud AudioProcessor error: {}", e);
      }
    }
    Err(e) => {
      error!(
        "SoundCloud: failed to init AudioProcessor (hint={:?}): {}",
        ext_hint, e
      );
    }
  }
}
