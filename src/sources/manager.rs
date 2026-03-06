use std::sync::Arc;

use super::{
    anghami::AnghamiSource,
    applemusic::AppleMusicSource,
    audiomack::AudiomackSource,
    audius::AudiusSource,
    bandcamp::BandcampSource,
    deezer::DeezerSource,
    flowery::FlowerySource,
    gaana::GaanaSource,
    google_tts::GoogleTtsSource,
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
    youtube::{YouTubeSource, YoutubeStreamContext, cipher::YouTubeCipherManager},
};
use crate::common::HttpClientPool;

/// Source Manager
pub struct SourceManager {
    pub sources: Vec<BoxedSource>,
    mirrors: Option<crate::config::server::MirrorsConfig>,
    pub youtube_cipher_manager: Option<Arc<YouTubeCipherManager>>,
    pub youtube_stream_ctx: Option<Arc<YoutubeStreamContext>>,
    pub http_pool: Arc<HttpClientPool>,
}

impl SourceManager {
    /// Create a new SourceManager with all available sources
    pub fn new(config: &crate::config::AppConfig) -> Self {
        let mut sources: Vec<BoxedSource> = Vec::new();
        let http_pool = Arc::new(HttpClientPool::new());

        // Register core sources
        Self::register_core_sources(&mut sources, config, &http_pool);

        // Register extra sources (TTS, local)
        Self::register_extra_sources(&mut sources, config, &http_pool);

        // YouTube special case: needs to expose stream context and cipher manager
        let yt_enabled = config.sources.youtube.as_ref().is_some_and(|c| c.enabled);
        let (youtube_cipher_manager, youtube_stream_ctx) = if yt_enabled {
            let yt_client = http_pool.get(None);
            let yt = YouTubeSource::new(config.sources.youtube.clone(), yt_client);
            (Some(yt.cipher_manager()), Some(yt.stream_context()))
        } else {
            (None, None)
        };

        Self {
            sources,
            mirrors: config.player.mirrors.clone(),
            youtube_cipher_manager,
            youtube_stream_ctx,
            http_pool,
        }
    }

    fn register_core_sources(
        sources: &mut Vec<BoxedSource>,
        config: &crate::config::AppConfig,
        http_pool: &Arc<HttpClientPool>,
    ) {
        macro_rules! register {
            ($enabled:expr, $name:literal, $proxy:expr, $ctor:expr) => {
                if $enabled {
                    if let Some(p) = &$proxy {
                        tracing::info!(
                            "Loading {} with proxy: {}",
                            $name,
                            p.url.as_ref().unwrap_or(&"enabled".to_owned())
                        );
                    }
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

        // YouTube (handled outside for special context)
        if config.sources.youtube.as_ref().is_some_and(|c| c.enabled) {
            tracing::info!("Loaded source: YouTube");
            let yt_client = http_pool.get(None);
            sources.push(Box::new(YouTubeSource::new(
                config.sources.youtube.clone(),
                yt_client,
            )));
        }

        let soundcloud_proxy = config
            .sources
            .soundcloud
            .as_ref()
            .and_then(|c| c.proxy.clone());
        register!(
            config
                .sources
                .soundcloud
                .as_ref()
                .is_some_and(|c| c.enabled),
            "SoundCloud",
            soundcloud_proxy,
            SoundCloudSource::new(
                config.sources.soundcloud.clone().unwrap(),
                http_pool.get(soundcloud_proxy.clone())
            )
        );

        register!(
            config.sources.spotify.as_ref().is_some_and(|c| c.enabled),
            "Spotify",
            None::<crate::config::HttpProxyConfig>,
            SpotifySource::new(config.sources.spotify.clone(), http_pool.get(None))
        );

        let jiosaavn_proxy = config
            .sources
            .jiosaavn
            .as_ref()
            .and_then(|c| c.proxy.clone());
        register!(
            config.sources.jiosaavn.as_ref().is_some_and(|c| c.enabled),
            "JioSaavn",
            jiosaavn_proxy,
            JioSaavnSource::new(
                config.sources.jiosaavn.clone(),
                http_pool.get(jiosaavn_proxy.clone())
            )
        );

        Self::register_deezer(sources, config, http_pool);

        let apple_proxy = config
            .sources
            .applemusic
            .as_ref()
            .and_then(|c| c.proxy.clone());
        register!(
            config
                .sources
                .applemusic
                .as_ref()
                .is_some_and(|c| c.enabled),
            "Apple Music",
            apple_proxy,
            AppleMusicSource::new(
                config.sources.applemusic.clone(),
                http_pool.get(apple_proxy.clone())
            )
        );

        let gaana_proxy = config.sources.gaana.as_ref().and_then(|c| c.proxy.clone());
        register!(
            config.sources.gaana.as_ref().is_some_and(|c| c.enabled),
            "Gaana",
            gaana_proxy,
            GaanaSource::new(
                config.sources.gaana.clone(),
                http_pool.get(gaana_proxy.clone())
            )
        );

        let tidal_proxy = config.sources.tidal.as_ref().and_then(|c| c.proxy.clone());
        register!(
            config.sources.tidal.as_ref().is_some_and(|c| c.enabled),
            "Tidal",
            tidal_proxy,
            TidalSource::new(
                config.sources.tidal.clone(),
                http_pool.get(tidal_proxy.clone())
            )
        );

        let audiomack_proxy = config
            .sources
            .audiomack
            .as_ref()
            .and_then(|c| c.proxy.clone());
        register!(
            config.sources.audiomack.as_ref().is_some_and(|c| c.enabled),
            "Audiomack",
            audiomack_proxy,
            AudiomackSource::new(
                config.sources.audiomack.clone(),
                http_pool.get(audiomack_proxy.clone())
            )
        );

        let pandora_proxy = config
            .sources
            .pandora
            .as_ref()
            .and_then(|c| c.proxy.clone());
        register!(
            config.sources.pandora.as_ref().is_some_and(|c| c.enabled),
            "Pandora",
            pandora_proxy,
            PandoraSource::new(
                config.sources.pandora.clone(),
                http_pool.get(pandora_proxy.clone())
            )
        );

        let qobuz_proxy = config.sources.qobuz.as_ref().and_then(|c| c.proxy.clone());
        if config.sources.qobuz.as_ref().is_some_and(|c| c.enabled) {
            let qobuz_token_provided = config
                .sources
                .qobuz
                .as_ref()
                .and_then(|c| c.user_token.as_ref())
                .is_some_and(|t| !t.is_empty());

            if !qobuz_token_provided {
                tracing::warn!(
                    "Qobuz user_token is missing; all playback will fall back to mirrors."
                );
            }

            register!(
                true,
                "Qobuz",
                qobuz_proxy,
                QobuzSource::new(config, http_pool.get(qobuz_proxy.clone()))
            );
        }

        let anghami_proxy = config
            .sources
            .anghami
            .as_ref()
            .and_then(|c| c.proxy.clone());
        register!(
            config.sources.anghami.as_ref().is_some_and(|c| c.enabled),
            "Anghami",
            anghami_proxy,
            AnghamiSource::new(config, http_pool.get(anghami_proxy.clone()))
        );

        let shazam_proxy = config.sources.shazam.as_ref().and_then(|c| c.proxy.clone());
        register!(
            config.sources.shazam.as_ref().is_some_and(|c| c.enabled),
            "Shazam",
            shazam_proxy,
            ShazamSource::new(config, http_pool.get(shazam_proxy.clone()))
        );

        let mixcloud_proxy = config
            .sources
            .mixcloud
            .as_ref()
            .and_then(|c| c.proxy.clone());
        register!(
            config.sources.mixcloud.as_ref().is_some_and(|c| c.enabled),
            "Mixcloud",
            mixcloud_proxy,
            MixcloudSource::new(
                config.sources.mixcloud.clone(),
                http_pool.get(mixcloud_proxy.clone())
            )
        );

        let bandcamp_proxy = config
            .sources
            .bandcamp
            .as_ref()
            .and_then(|c| c.proxy.clone());
        register!(
            config.sources.bandcamp.as_ref().is_some_and(|c| c.enabled),
            "Bandcamp",
            bandcamp_proxy,
            BandcampSource::new(
                config.sources.bandcamp.clone(),
                http_pool.get(bandcamp_proxy.clone())
            )
        );

        let audius_proxy = config.sources.audius.as_ref().and_then(|c| c.proxy.clone());
        register!(
            config.sources.audius.as_ref().is_some_and(|c| c.enabled),
            "Audius",
            audius_proxy,
            AudiusSource::new(
                config.sources.audius.clone(),
                http_pool.get(audius_proxy.clone())
            )
        );

        Self::register_yandex(sources, config, http_pool);

        if config.sources.http.as_ref().is_some_and(|c| c.enabled) {
            tracing::info!("Loaded source: http");
            sources.push(Box::new(HttpSource::new()));
        }
    }

    fn register_deezer(
        sources: &mut Vec<BoxedSource>,
        config: &crate::config::AppConfig,
        http_pool: &Arc<HttpClientPool>,
    ) {
        let (deezer_token_provided, deezer_key_provided) =
            if let Some(c) = config.sources.deezer.as_ref() {
                let arls_provided = c
                    .arls
                    .as_ref()
                    .is_some_and(|a| !a.is_empty() && a.iter().any(|s| !s.is_empty()));
                let key_provided = c
                    .master_decryption_key
                    .as_ref()
                    .is_some_and(|k| !k.is_empty());
                (arls_provided, key_provided)
            } else {
                (false, false)
            };

        if config.sources.deezer.as_ref().is_some_and(|c| c.enabled) {
            if !deezer_token_provided || !deezer_key_provided {
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
            } else {
                let proxy = config.sources.deezer.as_ref().and_then(|c| c.proxy.clone());
                let source = DeezerSource::new(
                    config.sources.deezer.clone().unwrap(),
                    http_pool.get(proxy.clone()),
                );

                match source {
                    Ok(src) => {
                        tracing::info!("Loaded source: Deezer");
                        sources.push(Box::new(src));
                    }
                    Err(e) => {
                        tracing::error!("Deezer source failed to initialize: {}", e);
                    }
                }
            }
        }
    }

    fn register_yandex(
        sources: &mut Vec<BoxedSource>,
        config: &crate::config::AppConfig,
        http_pool: &Arc<HttpClientPool>,
    ) {
        if let Some(c) = config.sources.yandexmusic.as_ref()
            && c.enabled
        {
            let token_provided = c.access_token.is_some();

            if !token_provided {
                tracing::warn!(
                    "Yandex Music source is enabled but the access_token is missing; it will be disabled."
                );
            } else {
                let proxy = c.proxy.clone();
                let source = YandexMusicSource::new(
                    config.sources.yandexmusic.clone(),
                    http_pool.get(proxy.clone()),
                );

                match source {
                    Ok(src) => {
                        tracing::info!("Loaded source: Yandex Music");
                        sources.push(Box::new(src));
                    }
                    Err(e) => {
                        tracing::error!("Yandex Music source failed to initialize: {}", e);
                    }
                }
            }
        }
    }

    fn register_extra_sources(
        sources: &mut Vec<BoxedSource>,
        config: &crate::config::AppConfig,
        _http_pool: &Arc<HttpClientPool>,
    ) {
        // TTS Sources
        if let Some(c) = config.sources.google_tts.as_ref()
            && c.enabled
        {
            tracing::info!("Loaded source: Google TTS");
            sources.push(Box::new(GoogleTtsSource::new(c.clone())));
        }

        if let Some(c) = config.sources.flowery.as_ref()
            && c.enabled
        {
            tracing::info!("Loaded source: Flowery");
            sources.push(Box::new(FlowerySource::new(c.clone())));
        }

        // Local Source
        if config.sources.local.as_ref().is_some_and(|c| c.enabled) {
            tracing::info!("Loaded source: local");
            sources.push(Box::new(LocalSource::new()));
        }
    }

    /// Load tracks using the first matching source
    pub async fn load(
        &self,
        identifier: &str,
        routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> crate::protocol::tracks::LoadResult {
        for source in &self.sources {
            if source.can_handle(identifier) {
                tracing::debug!(
                    "SourceManager: Loading '{}' with source: {}",
                    identifier,
                    source.name()
                );
                return source.load(identifier, routeplanner.clone()).await;
            }
        }

        tracing::debug!(
            "SourceManager: No source matched identifier: '{}'",
            identifier
        );
        crate::protocol::tracks::LoadResult::Empty {}
    }
    pub async fn load_search(
        &self,
        query: &str,
        types: &[String],
        routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<crate::protocol::tracks::SearchResult> {
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
        track_info: &crate::protocol::tracks::TrackInfo,
        routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<BoxedTrack> {
        let identifier = track_info.uri.as_deref().unwrap_or(&track_info.identifier);

        // 1. Try primary source resolution
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

        // 2. Fallback to mirrors if configured
        if let Some(mirrors) = &self.mirrors {
            return self
                .resolve_with_mirrors(track_info, identifier, mirrors, routeplanner)
                .await;
        }

        tracing::debug!("Failed to resolve playable track for: {}", identifier);
        None
    }

    async fn resolve_with_mirrors(
        &self,
        track_info: &crate::protocol::tracks::TrackInfo,
        identifier: &str,
        mirrors: &crate::config::server::MirrorsConfig,
        routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<BoxedTrack> {
        let isrc = track_info.isrc.as_deref().unwrap_or("");
        let query = format!("{} - {}", track_info.title, track_info.author);

        let original_source_name = self
            .sources
            .iter()
            .find(|s| s.can_handle(identifier))
            .map(|s| s.name());

        for provider in &mirrors.providers {
            if isrc.is_empty() && provider.contains("%ISRC%") {
                tracing::debug!("Skipping mirror provider '{}': track has no ISRC", provider);
                continue;
            }

            let resolved = provider.replace("%ISRC%", isrc).replace("%QUERY%", &query);

            if let Some(handling_source) = self.sources.iter().find(|s| s.can_handle(&resolved)) {
                if handling_source.is_mirror() {
                    tracing::warn!(
                        "Skipping mirror provider '{}': '{}' is a Mirror-type source",
                        resolved,
                        handling_source.name()
                    );
                    continue;
                }
                if Some(handling_source.name()) == original_source_name {
                    tracing::debug!(
                        "Skipping mirror provider '{}': would loop back to '{}'",
                        resolved,
                        handling_source.name()
                    );
                    continue;
                }
            }

            let res = match self.load(&resolved, routeplanner.clone()).await {
                crate::protocol::tracks::LoadResult::Track(t) => {
                    let id = t.info.uri.as_deref().unwrap_or(&t.info.identifier);
                    self.resolve_nested_track(id, routeplanner.clone()).await
                }
                crate::protocol::tracks::LoadResult::Search(tracks) => {
                    if let Some(first) = tracks.first() {
                        let id = first.info.uri.as_deref().unwrap_or(&first.info.identifier);
                        self.resolve_nested_track(id, routeplanner.clone()).await
                    } else {
                        None
                    }
                }
                _ => None,
            };

            if let Some(track) = res {
                return Some(track);
            }
        }

        None
    }

    async fn resolve_nested_track(
        &self,
        identifier: &str,
        routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<BoxedTrack> {
        for source in &self.sources {
            if source.can_handle(identifier)
                && let Some(track) = source.get_track(identifier, routeplanner.clone()).await
            {
                return Some(track);
            }
        }
        None
    }

    /// Get names of all registered sources
    pub fn source_names(&self) -> Vec<String> {
        self.sources.iter().map(|s| s.name().to_string()).collect()
    }
    pub fn get_proxy_config(&self, source_name: &str) -> Option<crate::config::HttpProxyConfig> {
        self.sources
            .iter()
            .find(|s| s.name() == source_name)
            .and_then(|s| s.get_proxy_config())
    }
}
