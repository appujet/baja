use std::net::IpAddr;

use flume::{Receiver, Sender};
use tracing::warn;

use crate::{
  audio::processor::{AudioProcessor, DecoderCommand},
  configs::HttpProxyConfig,
  sources::{gaana::crypto::decrypt_stream_path, plugin::PlayableTrack},
};

pub struct GaanaTrack {
  pub client: reqwest::Client,
  pub track_id: String,
  pub stream_quality: String,
  pub local_addr: Option<IpAddr>,
  pub proxy: Option<HttpProxyConfig>,
}

impl PlayableTrack for GaanaTrack {
  fn start_decoding(&self) -> (Receiver<i16>, Sender<DecoderCommand>) {
    let (tx, rx) = flume::bounded::<i16>(4096 * 4);
    let (cmd_tx, cmd_rx) = flume::unbounded::<DecoderCommand>();

    let track_id = self.track_id.clone();
    let client = self.client.clone();
    let quality = self.stream_quality.clone();
    let local_addr = self.local_addr;
    let proxy = self.proxy.clone();

    std::thread::spawn(move || {
      let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

      let hls_url =
        runtime.block_on(async { fetch_stream_url_internal(&client, &track_id, &quality).await });

      if let Some(url) = hls_url {
        let reader = if url.contains(".m3u8") || url.contains("/api/manifest/hls_") {
          crate::sources::youtube::hls::HlsReader::new(&url, local_addr, None, None, proxy.clone())
            .ok()
            .map(|r| Box::new(r) as Box<dyn symphonia::core::io::MediaSource>)
        } else {
          super::reader::GaanaReader::new(&url, local_addr, proxy.clone())
            .ok()
            .map(|r| Box::new(r) as Box<dyn symphonia::core::io::MediaSource>)
        };

        let ext_hint = if url.contains(".m3u8") || url.contains("/api/manifest/hls_") {
          Some("aac")
        } else {
          std::path::Path::new(&url)
            .extension()
            .and_then(|s| s.to_str())
        };

        if let Some(reader) = reader {
          match AudioProcessor::new(reader, ext_hint, tx, cmd_rx) {
            Ok(mut processor) => {
              if let Err(e) = processor.run() {
                tracing::error!("GaanaTrack audio processor error: {}", e);
              }
            }
            Err(e) => {
              tracing::error!("GaanaTrack failed to initialize processor: {}", e);
            }
          }
        } else {
          tracing::error!("GaanaTrack failed to create reader for {}", url);
        }
      } else {
        warn!("GaanaTrack: Failed to fetch stream URL for {}", track_id);
      }
    });

    (rx, cmd_tx)
  }
}

async fn fetch_stream_url_internal(
  client: &reqwest::Client,
  track_id: &str,
  quality: &str,
) -> Option<String> {
  let body = format!(
    "quality={}&track_id={}&stream_format=mp4",
    urlencoding::encode(quality),
    urlencoding::encode(track_id)
  );

  let resp = client
    .post("https://gaana.com/api/stream-url")
    .header("Content-Type", "application/x-www-form-urlencoded")
    .body(body)
    .send()
    .await
    .ok()?;

  if !resp.status().is_success() {
    return None;
  }

  let data: serde_json::Value = resp.json().await.ok()?;
  let encrypted_path = data
    .get("data")
    .and_then(|d| d.get("stream_path"))
    .and_then(|v| v.as_str())?;

  decrypt_stream_path(encrypted_path)
}
