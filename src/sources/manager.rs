use std::sync::Arc;

use super::{
    anghami::AnghamiSource,
    applemusic::AppleMusicSource,
    audiomack::AudiomackSource,
    audius::AudiusSource,
    bandcamp::BandcampSource,
    deezer::DeezerSource,
    gaana::GaanaSource,
    http::HttpSource,
    jiosaavn::JioSaavnSource,
    local::LocalSource,
    mixcloud::MixcloudSource,
    pandora::PandoraSource,
    plugin::{BoxedSource, BoxedTrack},
    qobuz::QobuzSource,
    shazam::ShazamSource,
    soundcloud::SoundCloudSource,
    spotify::SpotifySource,
    tidal::TidalSource,
    yandexmusic::YandexMusicSource,
    youtube::{YouTubeSource, cipher::YouTubeCipherManager},
};

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

        if config.sources.youtube {
            tracing::info!("Loaded source: YouTube");
            let yt = YouTubeSource::new(config.youtube.clone());
            youtube_cipher_manager = Some(yt.cipher_manager());
            sources.push(Box::new(yt));
        }

        register_source!(
            config.sources.soundcloud,
            "SoundCloud",
            SoundCloudSource::new(config.soundcloud.clone().unwrap_or_default())
        );

        register_source!(
            config.sources.spotify,
            "Spotify",
            SpotifySource::new(config.spotify.clone())
        );

        register_source!(
            config.sources.jiosaavn,
            "JioSaavn",
            JioSaavnSource::new(config.jiosaavn.clone())
        );

        let (deezer_token_provided, deezer_key_provided) = if let Some(c) = config.deezer.as_ref() {
            let arls_provided = c
                .arls
                .as_ref()
                .map(|a| !a.is_empty() && a.iter().any(|s| !s.is_empty()))
                .unwrap_or(false);
            let key_provided = c
                .master_decryption_key
                .as_ref()
                .map(|k| !k.is_empty())
                .unwrap_or(false);
            (arls_provided, key_provided)
        } else {
            (false, false)
        };
        if config.sources.deezer && (!deezer_token_provided || !deezer_key_provided) {
            let mut missing = Vec::new();
            if !deezer_token_provided {
                missing.push("arls");
            }
            if !deezer_key_provided {
                missing.push("master_decryption_key");
            }
            tracing::warn!(
                "Deezer source is enabled but {} {} missing; it will be disabled.",
                missing.join(" and "),
                if missing.len() > 1 { "are" } else { "is" }
            );
        }
        register_source!(
            config.sources.deezer && deezer_token_provided && deezer_key_provided,
            "Deezer",
            DeezerSource::new(config.deezer.clone().unwrap_or_default())
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
            config.sources.audiomack,
            "Audiomack",
            AudiomackSource::new(config.audiomack.clone())
        );

        register_source!(
            config.sources.pandora,
            "Pandora",
            PandoraSource::new(config.pandora.clone())
        );

        let qobuz_token_provided = config
            .qobuz
            .as_ref()
            .and_then(|c| c.user_token.as_ref())
            .map(|t| !t.is_empty())
            .unwrap_or(false);
        if config.sources.qobuz && !qobuz_token_provided {
            tracing::warn!("Qobuz user_token is missing; all playback will fall back to mirrors.");
        }
        register_source!(config.sources.qobuz, "Qobuz", QobuzSource::new(config));

        register_source!(
            config.sources.anghami,
            "Anghami",
            AnghamiSource::new(config)
        );

        register_source!(config.sources.shazam, "Shazam", ShazamSource::new(config));

        register_source!(
            config.sources.mixcloud,
            "Mixcloud",
            MixcloudSource::new(config.mixcloud.clone())
        );

        register_source!(
            config.sources.bandcamp,
            "Bandcamp",
            BandcampSource::new(config.bandcamp.clone())
        );

        register_source!(
            config.sources.audius,
            "Audius",
            AudiusSource::new(config.audius.clone())
        );

        let yandex_token_provided = config
            .yandexmusic
            .as_ref()
            .and_then(|c| c.access_token.as_ref())
            .is_some();
        if config.sources.yandexmusic && !yandex_token_provided {
            tracing::warn!(
                "Yandex Music source is enabled but the access_token is missing; it will be disabled."
            );
        }
        register_source!(
            config.sources.yandexmusic && yandex_token_provided,
            "Yandex Music",
            YandexMusicSource::new(config.yandexmusic.clone())
        );

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

            let provider_queries: Vec<String> = mirrors
                .providers
                .iter()
                .filter_map(|p| {
                    if isrc.is_empty() && p.contains("%ISRC%") {
                        None
                    } else {
                        Some(p.replace("%ISRC%", isrc).replace("%QUERY%", &query))
                    }
                })
                .collect();

            if !provider_queries.is_empty() {
                use futures::stream::{FuturesUnordered, StreamExt};

                let timeout_dur = std::time::Duration::from_millis(mirrors.timeout_ms);
                let this = self;
                let n = provider_queries.len();

                let mut futs: FuturesUnordered<_> = provider_queries
                    .into_iter()
                    .enumerate()
                    .map(|(p_idx, sq)| {
                        let rp = routeplanner.clone();
                        async move {
                            let res = tokio::time::timeout(timeout_dur, async move {
                                match this.load(&sq, rp.clone()).await {
                                    crate::api::tracks::LoadResult::Track(t) => {
                                        let id =
                                            t.info.uri.as_deref().unwrap_or(&t.info.identifier);
                                        this.resolve_nested_track(id, rp).await
                                    }
                                    crate::api::tracks::LoadResult::Search(tracks) => {
                                        if let Some(first) = tracks.first() {
                                            let id = first
                                                .info
                                                .uri
                                                .as_deref()
                                                .unwrap_or(&first.info.identifier);
                                            this.resolve_nested_track(id, rp).await
                                        } else {
                                            None
                                        }
                                    }
                                    _ => None,
                                }
                            })
                            .await
                            .unwrap_or(None);
                            (p_idx, res)
                        }
                    })
                    .collect();

                let mut slots: Vec<Option<Option<BoxedTrack>>> = (0..n).map(|_| None).collect();

                while let Some((idx, res)) = futs.next().await {
                    slots[idx] = Some(res);

                    for slot in slots.iter_mut() {
                        match slot {
                            Some(Some(_)) => return slot.take().flatten(),
                            Some(None) => continue,
                            None => break,
                        }
                    }
                }
            }
        }

        tracing::debug!("Failed to resolve playable track for: {}", identifier);
        None
    }

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
        self.sources
            .iter()
            .find(|s| s.name() == source_name)
            .and_then(|s| s.get_proxy_config())
    }
}
