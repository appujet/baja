use std::{net::IpAddr, sync::Arc};

use flume::{Receiver, Sender};
use tracing::{debug, error};

use crate::{
  audio::processor::{AudioProcessor, DecoderCommand},
  configs::HttpProxyConfig,
  sources::{
    plugin::PlayableTrack,
    youtube::{
      cipher::YouTubeCipherManager, clients::YouTubeClient, hls::HlsReader, oauth::YouTubeOAuth,
    },
  },
};

pub struct YoutubeTrack {
  pub identifier: String,
  pub clients: Vec<Arc<dyn YouTubeClient>>,
  pub oauth: Arc<YouTubeOAuth>,
  pub cipher_manager: Arc<YouTubeCipherManager>,
  pub visitor_data: Option<String>,
  pub local_addr: Option<IpAddr>,
  pub proxy: Option<HttpProxyConfig>,
}

impl PlayableTrack for YoutubeTrack {
  fn start_decoding(
    &self,
  ) -> (
    Receiver<Vec<i16>>,
    Sender<DecoderCommand>,
    flume::Receiver<String>,
  ) {
    let (tx, rx) = flume::bounded::<Vec<i16>>(64);
    let (cmd_tx, cmd_rx) = flume::unbounded::<DecoderCommand>();
    let (err_tx, err_rx) = flume::bounded::<String>(1);

    // Prepare data for the decoding thread
    let identifier = self.identifier.clone();
    let clients = self.clients.clone();
    let oauth = self.oauth.clone();
    let cipher_manager = self.cipher_manager.clone();
    let visitor_data = self.visitor_data.clone();
    let local_addr = self.local_addr;
    let proxy = self.proxy.clone();

    let handle = tokio::runtime::Handle::current();
    std::thread::spawn(move || {
      let context = serde_json::json!({ "visitorData": visitor_data });
      let mut success = false;

      for client in &clients {
        let client_name = client.name();
        debug!(
          "YoutubeTrack: Resolving '{}' using {}",
          identifier, client_name
        );

        let playback_result = handle.block_on(async {
          client
            .get_track_url(&identifier, &context, cipher_manager.clone(), oauth.clone())
            .await
        });

        let url = match playback_result {
          Ok(Some(u)) => u,
          Ok(None) => {
            debug!("YoutubeTrack: {} returned no stream URL", client_name);
            continue;
          }
          Err(e) => {
            debug!("YoutubeTrack: {} failed to resolve: {}", client_name, e);
            continue;
          }
        };

        debug!(
          "YoutubeTrack: Resolved stream URL via {}: {}",
          client_name, url
        );

        // 1. Initialize the appropriate MediaSource reader
        let reader: Box<dyn symphonia::core::io::MediaSource> =
          if url.contains(".m3u8") || url.contains("/playlist") {
            let player_url = if url.contains("youtube.com") {
              Some(url.clone())
            } else {
              None
            };

            match HlsReader::new(
              &url,
              local_addr,
              Some(cipher_manager.clone()),
              player_url,
              proxy.clone(),
            ) {
              Ok(r) => Box::new(r),
              Err(e) => {
                error!(
                  "YoutubeTrack: HlsReader initialization failed for {}: {}",
                  client_name, e
                );
                continue;
              }
            }
          } else {
            match super::reader::YoutubeReader::new(&url, local_addr, proxy.clone()) {
              Ok(r) => Box::new(r),
              Err(e) => {
                error!(
                  "YoutubeTrack: YoutubeReader initialization failed for {}: {}",
                  client_name, e
                );
                continue;
              }
            }
          };

        // 2. Identify the likely codec format for Symphonia's demuxer
        let kind = if url.contains(".m3u8") || url.contains("/hls_") {
          Some(crate::common::types::AudioKind::Aac)
        } else if url.contains("itag=251") || url.contains("mime=audio/webm") {
          Some(crate::common::types::AudioKind::Webm)
        } else if url.contains("itag=140") || url.contains("mime=audio/mp4") {
          Some(crate::common::types::AudioKind::Mp4)
        } else {
          std::path::Path::new(&url)
            .extension()
            .and_then(|s| s.to_str())
            .and_then(crate::common::types::AudioKind::from_ext)
        };

        // 3. Initialize AudioProcessor and start decoding session
        match AudioProcessor::new(
          reader,
          kind,
          tx.clone(),
          cmd_rx.clone(),
          Some(err_tx.clone()),
        ) {
          Ok(mut processor) => {
            debug!(
              "YoutubeTrack: Playback session started for {} using {}",
              identifier,
              client_name
            );
            success = true;
            if let Err(e) = processor.run() {
              error!("YoutubeTrack: Decoding session finished with error: {}", e);
            }
            break; // Successfully played/finished, stop trying other clients
          }
          Err(e) => {
            error!(
              "YoutubeTrack: AudioProcessor initialization failed with {}: {}",
              client_name, e
            );
            continue;
          }
        }
      }

      if !success {
        error!(
          "YoutubeTrack: All configured playback clients failed for {}",
          identifier
        );
      }
    });

    (rx, cmd_tx, err_rx)
  }
}
