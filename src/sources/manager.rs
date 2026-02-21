use std::sync::Arc;

use tracing::info;

use super::{
  applemusic::manager::AppleMusicSource,
  deezer::DeezerSource,
  gaana::GaanaSource,
  http::HttpSource,
  jiosaavn::JioSaavnSource,
  plugin::{BoxedSource, BoxedTrack, PlayableTrack},
  soundcloud::SoundCloudSource,
  spotify::manager::SpotifySource,
  tidal::TidalSource,
  youtube::{YouTubeSource, cipher::YouTubeCipherManager},
};
use crate::audio::processor::DecoderCommand;

/// Source Manager
pub struct SourceManager {
  sources: Vec<BoxedSource>,
  mirrors: Option<crate::configs::MirrorsConfig>,
  pub youtube_cipher_manager: Option<Arc<YouTubeCipherManager>>,
}

impl SourceManager {
  /// Create a new SourceManager with all available sources
  pub fn new(config: &crate::configs::Config) -> Self {
    let mut sources: Vec<BoxedSource> = Vec::new();
    let mut youtube_cipher_manager = None;

    // Register all sources
    if config.sources.jiosaavn {
      info!("Registering JioSaavn source");
      sources.push(Box::new(JioSaavnSource::new(config.jiosaavn.clone())));
    }
    if config.sources.deezer {
      info!("Registering Deezer source");
      sources.push(Box::new(
        DeezerSource::new(config.deezer.clone().unwrap_or_default())
          .expect("Failed to create Deezer source"),
      ));
    }
    if config.sources.youtube {
      info!("Registering YouTube source");
      let yt = YouTubeSource::new(config.youtube.clone());
      youtube_cipher_manager = Some(yt.cipher_manager());
      sources.push(Box::new(yt));
    }
    if config.sources.spotify {
      info!("Registering Spotify source");
      sources.push(Box::new(SpotifySource::new(config.spotify.clone())));
    }
    if config.sources.applemusic {
      info!("Registering Apple Music source");
      sources.push(Box::new(AppleMusicSource::new(config.applemusic.clone())));
    }
    if config.sources.gaana {
      info!("Registering Gaana source");
      sources.push(Box::new(GaanaSource::new(config.gaana.clone())));
    }
    if config.sources.tidal {
      info!("Registering Tidal source");
      sources.push(Box::new(TidalSource::new(config.tidal.clone())));
    }
    if config.sources.soundcloud {
      info!("Registering SoundCloud source");
      sources.push(Box::new(SoundCloudSource::new(
        config.soundcloud.clone().unwrap_or_default(),
      )));
    }
    if config.sources.http {
      info!("Registering HTTP source");
      sources.push(Box::new(HttpSource::new()));
    }

    Self {
      sources,
      mirrors: config.mirrors.clone(),
      youtube_cipher_manager,
    }
  }

  /// Load tracks using the first matching source
  pub async fn load(
    &self,
    identifier: &str,
    routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
  ) -> crate::api::tracks::LoadResult {
    // Try each source in order
    for source in &self.sources {
      if source.can_handle(identifier) {
        tracing::debug!("Loading '{}' with source: {}", identifier, source.name());
        return source.load(identifier, routeplanner.clone()).await;
      }
    }

    tracing::warn!("No source could handle identifier: {}", identifier);
    crate::api::tracks::LoadResult::Empty {}
  }
  pub async fn load_search(
    &self,
    query: &str,
    types: &[String],
    routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
  ) -> Option<crate::api::tracks::SearchResult> {
    // Try each source in order
    for source in &self.sources {
      if source.can_handle(query) {
        tracing::debug!("Loading search '{}' with source: {}", query, source.name());
        // Call load_search on the candidate source
        return source.load_search(query, types, routeplanner.clone()).await;
      }
    }

    tracing::warn!("No source could handle search query: {}", query);
    None
  }

  pub async fn get_track(
    &self,
    track_info: &crate::api::tracks::TrackInfo,
    routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
  ) -> Option<BoxedTrack> {
    let identifier = track_info.uri.as_deref().unwrap_or(&track_info.identifier);

    for source in &self.sources {
      if source.can_handle(identifier) {
        tracing::debug!(
          "Resolving playable track for '{}' with source: {}",
          identifier,
          source.name()
        );

        if let Some(track) = source.get_track(identifier, routeplanner.clone()).await {
          return Some(track);
        }
        break;
      }
    }

    if let Some(mirrors) = &self.mirrors {
      let isrc = track_info.isrc.as_deref().unwrap_or("");
      let query = format!("{} - {}", track_info.title, track_info.author);

      if isrc.is_empty() {
        tracing::debug!("Track has no ISRC");
      }

      for provider in &mirrors.providers {
        let search_query = provider.replace("%ISRC%", isrc).replace("%QUERY%", &query);

        if isrc.is_empty() && provider.contains("%ISRC%") {
          continue;
        }

        tracing::debug!("Attempting mirror provider: {}", search_query);

        match self.load(&search_query, routeplanner.clone()).await {
          crate::api::tracks::LoadResult::Track(track) => {
            let nested_id = track.info.uri.as_deref().unwrap_or(&track.info.identifier);
            if let Some(playable) = self
              .resolve_nested_track(nested_id, routeplanner.clone())
              .await
            {
              tracing::debug!(
                "Mirror success: {} -> {}",
                search_query,
                track.info.identifier
              );
              return Some(playable);
            }
          }
          crate::api::tracks::LoadResult::Search(tracks) => {
            if let Some(first_track) = tracks.first() {
              let nested_id = first_track
                .info
                .uri
                .as_deref()
                .unwrap_or(&first_track.info.identifier);
              if let Some(playable) = self
                .resolve_nested_track(nested_id, routeplanner.clone())
                .await
              {
                tracing::debug!(
                  "Mirror success (search): {} -> {}",
                  search_query,
                  first_track.info.identifier
                );
                return Some(playable);
              }
            }
          }
          _ => {}
        }
      }
    }

    tracing::warn!("Failed to resolve playable track for: {}", identifier);
    None
  }

  /// Helper to resolve a nested ID found via mirror search
  async fn resolve_nested_track(
    &self,
    identifier: &str,
    routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
  ) -> Option<BoxedTrack> {
    for source in &self.sources {
      if source.can_handle(identifier) {
        if let Some(track) = source.get_track(identifier, routeplanner.clone()).await {
          return Some(track);
        }
      }
    }
    None
  }

  /// Get names of all registered sources
  pub fn source_names(&self) -> Vec<String> {
    self.sources.iter().map(|s| s.name().to_string()).collect()
  }
  pub fn get_proxy_config(&self, source_name: &str) -> Option<crate::configs::HttpProxyConfig> {
    self
      .sources
      .iter()
      .find(|s| s.name() == source_name)
      .and_then(|s| s.get_proxy_config())
  }
}

pub struct MirroredTrack {
  pub query: String,
  pub source_manager: Arc<SourceManager>,
}

impl PlayableTrack for MirroredTrack {
  fn start_decoding(&self) -> (flume::Receiver<i16>, flume::Sender<DecoderCommand>) {
    let (tx, rx) = flume::bounded::<i16>(4096 * 4);
    let (cmd_tx, cmd_rx) = flume::unbounded::<DecoderCommand>();

    let query = self.query.clone();
    let manager = self.source_manager.clone();

    std::thread::spawn(move || {
      let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

      runtime.block_on(async {
        if let crate::api::tracks::LoadResult::Search(tracks) = manager.load(&query, None).await {
          if let Some(first) = tracks.first() {
            if let Some(playable) = manager.get_track(&first.info, None).await {
              let (inner_rx, inner_cmd_tx) = playable.start_decoding();

              // Proxy commands
              let cmd_tx_clone = inner_cmd_tx.clone();
              std::thread::spawn(move || {
                while let Ok(cmd) = cmd_rx.recv() {
                  let _ = cmd_tx_clone.send(cmd);
                }
              });

              // Proxy samples
              while let Ok(sample) = inner_rx.recv() {
                if tx.send(sample).is_err() {
                  break;
                }
              }
            }
          }
        }
      });
    });

    (rx, cmd_tx)
  }
}
