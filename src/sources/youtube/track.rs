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
  fn start_decoding(&self) -> (Receiver<i16>, Sender<DecoderCommand>) {
    let (tx, rx) = flume::bounded::<i16>(4096 * 4);
    let (cmd_tx, cmd_rx) = flume::unbounded::<DecoderCommand>();

    // Clone needed data for the thread
    let identifier = self.identifier.clone();
    let clients = self.clients.clone();
    let oauth = self.oauth.clone();
    let cipher_manager = self.cipher_manager.clone();
    let visitor_data = self.visitor_data.clone();
    let local_addr = self.local_addr;
    let proxy = self.proxy.clone();

    // Use a persistent reference for helper calls
    let track_ref = Arc::new(YoutubeTrack {
      identifier: identifier.clone(),
      clients,
      oauth,
      cipher_manager,
      visitor_data: visitor_data.clone(),
      local_addr,
      proxy: proxy.clone(),
    });

    std::thread::spawn(move || {
      let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

      let context = if let Some(vd) = &visitor_data {
        serde_json::json!({ "visitorData": vd })
      } else {
        serde_json::json!({})
      };

      let mut success = false;

      for client in &track_ref.clients {
        debug!(
          "YoutubeTrack: Attempting playback for '{}' with {}",
          identifier,
          client.name()
        );

        let playback_url = runtime.block_on(async {
          match client
            .get_track_url(
              &identifier,
              &context,
              track_ref.cipher_manager.clone(),
              track_ref.oauth.clone(),
            )
            .await
          {
            Ok(Some(url)) => Some(url),
            _ => None,
          }
        });

        let url = match playback_url {
          Some(u) => u,
          None => {
            debug!(
              "YoutubeTrack: Client {} failed to resolve URL",
              client.name()
            );
            continue;
          }
        };

        debug!("YoutubeTrack: Resolved URL using {}", client.name());

        let custom_reader: Option<Box<dyn symphonia::core::io::MediaSource>> =
          if url.contains(".m3u8") || url.contains("/playlist") {
            let player_url = if url.contains("youtube.com") {
              Some(url.clone())
            } else {
              None
            };
            HlsReader::new(
              &url,
              local_addr,
              Some(track_ref.cipher_manager.clone()),
              player_url,
              proxy.clone(),
            )
            .ok()
            .map(|r| Box::new(r) as Box<dyn symphonia::core::io::MediaSource>)
          } else {
            None
          };

        let reader = match custom_reader {
          Some(r) => r,
          None => match super::reader::YoutubeReader::new(&url, local_addr, proxy.clone()) {
            Ok(r) => Box::new(r) as Box<dyn symphonia::core::io::MediaSource>,
            Err(e) => {
              error!(
                "YoutubeTrack: Failed to open YoutubeReader for {}: {}",
                client.name(),
                e
              );
              continue;
            }
          },
        };

        let ext_hint = if url.contains(".m3u8") || url.contains("/api/manifest/hls_") {
          Some("aac")
        } else if url.contains("itag=251")
          || url.contains("itag=250")
          || url.contains("mime=audio/webm")
        {
          Some("webm")
        } else if url.contains("itag=140") || url.contains("mime=audio/mp4") {
          Some("mp4")
        } else {
          std::path::Path::new(&url)
            .extension()
            .and_then(|s| s.to_str())
        };

        match AudioProcessor::new(reader, ext_hint, tx.clone(), cmd_rx.clone()) {
          Ok(mut processor) => {
            debug!(
              "YoutubeTrack: Playback started successfully with {}",
              client.name()
            );
            success = true;
            if let Err(e) = processor.run() {
              error!("YoutubeTrack: Decoding session failed: {}", e);
            }
            break; // Stop trying other clients if we successfully started and then finished/failed
          }
          Err(e) => {
            error!(
              "YoutubeTrack: Initialization failed with {}: {}. Trying next client...",
              client.name(),
              e
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

    (rx, cmd_tx)
  }
}
