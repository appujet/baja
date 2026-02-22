use std::sync::Arc;

use super::{
  anghami::AnghamiSource,
  applemusic::AppleMusicSource,
  audiomack::manager::AudiomackSource,
  deezer::DeezerSource,
  gaana::GaanaSource,
  http::HttpSource,
  jiosaavn::JioSaavnSource,
  local::LocalSource,
  pandora::PandoraSource,
  plugin::{BoxedSource, BoxedTrack, PlayableTrack},
  qobuz::QobuzSource,
  shazam::ShazamSource,
  soundcloud::SoundCloudSource,
  spotify::SpotifySource,
  tidal::TidalSource,
  youtube::{YouTubeSource, cipher::YouTubeCipherManager},
};
use crate::audio::processor::DecoderCommand;

/// Source Manager
pub struct SourceManager {
  pub sources: Vec<BoxedSource>,
  mirrors: Option<crate::configs::MirrorsConfig>,
  pub youtube_cipher_manager: Option<Arc<YouTubeCipherManager>>,
}

impl SourceManager {
  /// Create a new SourceManager with all available sources
  pub fn new(config: &crate::configs::Config) -> Self {
    let mut sources: Vec<BoxedSource> = Vec::new();
    let mut youtube_cipher_manager = None;

    // Register all sources using a macro for better scalability (M3)
    macro_rules! register_source {
      ($enabled:expr, $name:literal, $ctor:expr) => {
        if $enabled {
          match $ctor {
            Ok(src) => {
              tracing::info!("Loaded source: {}", $name);
              sources.push(Box::new(src));
            }
            Err(e) => {
              tracing::error!("{} source failed to initialize: {}", $name, e);
            }
          }
        }
      };
    }

    register_source!(
      config.sources.jiosaavn,
      "JioSaavn",
      JioSaavnSource::new(config.jiosaavn.clone())
    );
    register_source!(
      config.sources.deezer,
      "Deezer",
      DeezerSource::new(config.deezer.clone().unwrap_or_default())
    );
    register_source!(
      config.sources.spotify,
      "Spotify",
      SpotifySource::new(config.spotify.clone())
    );
    register_source!(
      config.sources.applemusic,
      "Apple Music",
      AppleMusicSource::new(config.applemusic.clone())
    );
    register_source!(
      config.sources.gaana,
      "Gaana",
      GaanaSource::new(config.gaana.clone())
    );
    register_source!(
      config.sources.tidal,
      "Tidal",
      TidalSource::new(config.tidal.clone())
    );
    register_source!(
      config.sources.soundcloud,
      "SoundCloud",
      SoundCloudSource::new(config.soundcloud.clone().unwrap_or_default())
    );
    register_source!(
      config.sources.audiomack,
      "Audiomack",
      AudiomackSource::new(config.audiomack.clone())
    );
    register_source!(
      config.sources.pandora,
      "Pandora",
      PandoraSource::new(config.pandora.clone())
    );
    register_source!(config.sources.qobuz, "Qobuz", QobuzSource::new(config));
    register_source!(
      config.sources.anghami,
      "Anghami",
      AnghamiSource::new(config)
    );
    register_source!(
      config.sources.shazam,
      "Shazam",
      ShazamSource::new(config)
    );

    if config.sources.youtube {
      tracing::info!("Loaded source: YouTube");
      let yt = YouTubeSource::new(config.youtube.clone());
      youtube_cipher_manager = Some(yt.cipher_manager());
      sources.push(Box::new(yt));
    }

    if config.sources.http {
      tracing::info!("Loaded source: http");
      sources.push(Box::new(HttpSource::new()));
    }

    if config.sources.local {
      tracing::info!("Loaded source: local");
      sources.push(Box::new(LocalSource::new()));
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
        tracing::trace!("Loading '{}' with source: {}", identifier, source.name());
        return source.load(identifier, routeplanner.clone()).await;
      }
    }

    tracing::debug!("No source could handle identifier: {}", identifier);
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
        tracing::trace!("Loading search '{}' with source: {}", query, source.name());
        // Call load_search on the candidate source
        return source.load_search(query, types, routeplanner.clone()).await;
      }
    }

    tracing::debug!("No source could handle search query: {}", query);
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
        tracing::trace!(
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
        tracing::trace!("Track has no ISRC");
      }

      for provider in &mirrors.providers {
        let search_query = provider.replace("%ISRC%", isrc).replace("%QUERY%", &query);

        if isrc.is_empty() && provider.contains("%ISRC%") {
          continue;
        }

        tracing::trace!("Attempting mirror provider: {}", search_query);

        match self.load(&search_query, routeplanner.clone()).await {
          crate::api::tracks::LoadResult::Track(track) => {
            let nested_id = track.info.uri.as_deref().unwrap_or(&track.info.identifier);
            if let Some(playable) = self
              .resolve_nested_track(nested_id, routeplanner.clone())
              .await
            {
              tracing::trace!(
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
                tracing::trace!(
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

    tracing::debug!("Failed to resolve playable track for: {}", identifier);
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
  fn start_decoding(
    &self,
  ) -> (
    flume::Receiver<Vec<i16>>,
    flume::Sender<DecoderCommand>,
    flume::Receiver<String>,
  ) {
    let (tx, rx) = flume::bounded::<Vec<i16>>(64);
    let (cmd_tx, cmd_rx) = flume::unbounded::<DecoderCommand>();
    let (err_tx, err_rx) = flume::bounded::<String>(1);

    let query = self.query.clone();
    let manager = self.source_manager.clone();

    tokio::spawn(async move {
      if let crate::api::tracks::LoadResult::Search(tracks) = manager.load(&query, None).await {
        if let Some(first) = tracks.first() {
          if let Some(playable) = manager.get_track(&first.info, None).await {
            let (inner_rx, inner_cmd_tx, inner_err_rx) = playable.start_decoding();

            // Proxy commands: forward outer cmd_rx -> inner decoder
            let cmd_rx_task = cmd_rx.clone();
            let inner_cmd_tx_task = inner_cmd_tx.clone();
            tokio::spawn(async move {
              while let Ok(cmd) = cmd_rx_task.recv_async().await {
                if inner_cmd_tx_task.send(cmd).is_err() {
                  break;
                }
              }
            });

            // Proxy errors
            let err_tx_task = err_tx.clone();
            tokio::spawn(async move {
              if let Ok(err) = inner_err_rx.recv_async().await {
                let _ = err_tx_task.send(err);
              }
            });

            // Proxy PCM samples
            while let Ok(sample) = inner_rx.recv_async().await {
              if tx.send(sample).is_err() {
                break;
              }
            }
          }
        }
      }
    });

    (rx, cmd_tx, err_rx)
  }
}
