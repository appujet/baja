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
  pub bitrate_bps: u64,
  pub local_addr: Option<IpAddr>,
  pub proxy: Option<HttpProxyConfig>,
}

impl PlayableTrack for SoundCloudTrack {
  fn start_decoding(
    &self,
  ) -> (
    Receiver<i16>,
    Sender<DecoderCommand>,
    flume::Receiver<String>,
  ) {
    let (tx, rx) = flume::bounded::<i16>(4096 * 4);
    let (cmd_tx, cmd_rx) = flume::unbounded::<DecoderCommand>();
    let (err_tx, err_rx) = flume::bounded::<String>(1);

    let stream_url = self.stream_url.clone();
    let kind = self.kind.clone();
    let bitrate_bps = self.bitrate_bps;
    let local_addr = self.local_addr;
    let proxy = self.proxy.clone();

    std::thread::spawn(move || {
      match kind {
        SoundCloudStreamKind::ProgressiveMp3 => {
          let reader = match super::reader::SoundCloudReader::new(&stream_url, local_addr, proxy) {
            Ok(r) => Box::new(r) as Box<dyn symphonia::core::io::MediaSource>,
            Err(e) => {
              error!("SoundCloud progressive MP3: failed to open stream: {}", e);
              return;
            }
          };
          run_processor(
            reader,
            Some(crate::common::types::AudioKind::Mp3),
            tx,
            cmd_rx,
            err_tx,
          );
        }

        SoundCloudStreamKind::ProgressiveAac => {
          let reader = match super::reader::SoundCloudReader::new(&stream_url, local_addr, proxy) {
            Ok(r) => Box::new(r) as Box<dyn symphonia::core::io::MediaSource>,
            Err(e) => {
              error!("SoundCloud progressive AAC: failed to open stream: {}", e);
              return;
            }
          };
          run_processor(
            reader,
            Some(crate::common::types::AudioKind::Mp4),
            tx,
            cmd_rx,
            err_tx,
          );
        }

        SoundCloudStreamKind::HlsOpus => {
          let reader = match super::reader::SoundCloudHlsReader::new(
            &stream_url,
            bitrate_bps,
            local_addr,
            proxy,
          ) {
            Ok(r) => Box::new(r) as Box<dyn symphonia::core::io::MediaSource>,
            Err(e) => {
              error!(
                "SoundCloud HLS Opus: failed to init SoundCloudHlsReader: {}",
                e
              );
              return;
            }
          };
          run_processor(
            reader,
            Some(crate::common::types::AudioKind::Opus),
            tx,
            cmd_rx,
            err_tx,
          );
        }

        SoundCloudStreamKind::HlsMp3 => {
          let reader = match super::reader::SoundCloudHlsReader::new(
            &stream_url,
            bitrate_bps,
            local_addr,
            proxy,
          ) {
            Ok(r) => Box::new(r) as Box<dyn symphonia::core::io::MediaSource>,
            Err(e) => {
              error!(
                "SoundCloud HLS MP3: failed to init SoundCloudHlsReader: {}",
                e
              );
              return;
            }
          };
          run_processor(
            reader,
            Some(crate::common::types::AudioKind::Mp3),
            tx,
            cmd_rx,
            err_tx,
          );
        }

        SoundCloudStreamKind::HlsAac => {
          let reader = match super::reader::SoundCloudHlsReader::new(
            &stream_url,
            bitrate_bps,
            local_addr,
            proxy,
          ) {
            Ok(r) => Box::new(r) as Box<dyn symphonia::core::io::MediaSource>,
            Err(e) => {
              error!(
                "SoundCloud HLS AAC: failed to init SoundCloudHlsReader: {}",
                e
              );
              return;
            }
          };
          // Hint as "aac" so symphonia knows what to expect from ADTS stream.
          run_processor(
            reader,
            Some(crate::common::types::AudioKind::Aac),
            tx,
            cmd_rx,
            err_tx,
          );
        }
      }
    });

    (rx, cmd_tx, err_rx)
  }
}

fn run_processor(
  reader: Box<dyn symphonia::core::io::MediaSource>,
  kind: Option<crate::common::types::AudioKind>,
  tx: flume::Sender<i16>,
  cmd_rx: flume::Receiver<DecoderCommand>,
  err_tx: flume::Sender<String>,
) {
  match AudioProcessor::new(reader, kind, tx, cmd_rx, Some(err_tx)) {
    Ok(mut p) => {
      if let Err(e) = p.run() {
        error!("SoundCloud AudioProcessor error: {}", e);
      }
    }
    Err(e) => {
      error!(
        "SoundCloud: failed to init AudioProcessor (kind={:?}): {}",
        kind, e
      );
    }
  }
}
