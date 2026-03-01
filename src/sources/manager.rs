use std::sync::Arc;

use super::{
    anghami::AnghamiSource,
    amazonmusic::AmazonMusicSource,
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
use crate::common::HttpClientPool;

/// Source Manager
pub struct SourceManager {
    pub sources: Vec<BoxedSource>,
    mirrors: Option<crate::configs::MirrorsConfig>,
    pub youtube_cipher_manager: Option<Arc<YouTubeCipherManager>>,
    pub http_pool: Arc<HttpClientPool>,
}

impl SourceManager {
    /// Create a new SourceManager with all available sources
    pub fn new(config: &crate::configs::Config) -> Self {
        let mut sources: Vec<BoxedSource> = Vec::new();
        let mut youtube_cipher_manager = None;
        let http_pool = Arc::new(HttpClientPool::new());

        // Register all sources using a macro for better scalability (M3)
        macro_rules! register_source {
            ($enabled:expr, $name:literal, $proxy:expr, $ctor:expr) => {
                if $enabled {
                    if let Some(p) = &$proxy {
                        tracing::info!(
                            "Loading {} with proxy: {}",
                            $name,
                            p.url.as_ref().unwrap_or(&"enabled".to_string())
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

        if config.sources.youtube {
            tracing::info!("Loaded source: YouTube");
            // YouTube doesn't currently define a proxy config in its config,
            // so we pass None to the pool to get a shared direct-connect client.
            let yt_client = http_pool.get(None);
            let yt = YouTubeSource::new(config.youtube.clone(), yt_client);
            youtube_cipher_manager = Some(yt.cipher_manager());
            sources.push(Box::new(yt));
        }

        let soundcloud_proxy = config.soundcloud.as_ref().and_then(|c| c.proxy.clone());
        register_source!(
            config.sources.soundcloud,
            "SoundCloud",
            soundcloud_proxy,
            SoundCloudSource::new(
                config.soundcloud.clone().unwrap_or_default(),
                http_pool.get(soundcloud_proxy.clone())
            )
        );

        // Spotify currently doesn't define a proxy in its config, but we'll use a direct client
        register_source!(
            config.sources.spotify,
            "Spotify",
            None::<crate::configs::HttpProxyConfig>,
            SpotifySource::new(config.spotify.clone(), http_pool.get(None))
        );

        let jiosaavn_proxy = config.jiosaavn.as_ref().and_then(|c| c.proxy.clone());
        register_source!(
            config.sources.jiosaavn,
            "JioSaavn",
            jiosaavn_proxy,
            JioSaavnSource::new(config.jiosaavn.clone(), http_pool.get(jiosaavn_proxy.clone()))
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
        let deezer_proxy = config.deezer.as_ref().and_then(|c| c.proxy.clone());
        register_source!(
            config.sources.deezer && deezer_token_provided && deezer_key_provided,
            "Deezer",
            deezer_proxy,
            DeezerSource::new(
                config.deezer.clone().unwrap_or_default(),
                http_pool.get(deezer_proxy.clone())
            )
        );

        let apple_proxy = config.applemusic.as_ref().and_then(|c| c.proxy.clone());
        register_source!(
            config.sources.applemusic,
            "Apple Music",
            apple_proxy,
            AppleMusicSource::new(
                config.applemusic.clone(),
                http_pool.get(apple_proxy.clone())
            )
        );

        let amazon_proxy = config.amazonmusic.as_ref().and_then(|c| c.proxy.clone());
        register_source!(
            config.sources.amazonmusic,
            "Amazon Music",
            amazon_proxy,
            AmazonMusicSource::new(
                config.amazonmusic.clone(),
                http_pool.get(amazon_proxy.clone())
            )
        );

        let gaana_proxy = config.gaana.as_ref().and_then(|c| c.proxy.clone());
        register_source!(
            config.sources.gaana,
            "Gaana",
            gaana_proxy,
            GaanaSource::new(config.gaana.clone(), http_pool.get(gaana_proxy.clone()))
        );

        let tidal_proxy = config.tidal.as_ref().and_then(|c| c.proxy.clone());
        register_source!(
            config.sources.tidal,
            "Tidal",
            tidal_proxy,
            TidalSource::new(config.tidal.clone(), http_pool.get(tidal_proxy.clone()))
        );

        let audiomack_proxy = config.audiomack.as_ref().and_then(|c| c.proxy.clone());
        register_source!(
            config.sources.audiomack,
            "Audiomack",
            audiomack_proxy,
            AudiomackSource::new(
                config.audiomack.clone(),
                http_pool.get(audiomack_proxy.clone())
            )
        );

        let pandora_proxy = config.pandora.as_ref().and_then(|c| c.proxy.clone());
        register_source!(
            config.sources.pandora,
            "Pandora",
            pandora_proxy,
            PandoraSource::new(config.pandora.clone(), http_pool.get(pandora_proxy.clone()))
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
        let qobuz_proxy = config.qobuz.as_ref().and_then(|c| c.proxy.clone());
        register_source!(
            config.sources.qobuz,
            "Qobuz",
            qobuz_proxy,
            QobuzSource::new(config, http_pool.get(qobuz_proxy.clone()))
        );

        let anghami_proxy = config.anghami.as_ref().and_then(|c| c.proxy.clone());
        register_source!(
            config.sources.anghami,
            "Anghami",
            anghami_proxy,
            AnghamiSource::new(config, http_pool.get(anghami_proxy.clone()))
        );

        let shazam_proxy = config.shazam.as_ref().and_then(|c| c.proxy.clone());
        register_source!(
            config.sources.shazam,
            "Shazam",
            shazam_proxy,
            ShazamSource::new(config, http_pool.get(shazam_proxy.clone()))
        );

        let mixcloud_proxy = config.mixcloud.as_ref().and_then(|c| c.proxy.clone());
        register_source!(
            config.sources.mixcloud,
            "Mixcloud",
            mixcloud_proxy,
            MixcloudSource::new(
                config.mixcloud.clone(),
                http_pool.get(mixcloud_proxy.clone())
            )
        );

        let bandcamp_proxy = config.bandcamp.as_ref().and_then(|c| c.proxy.clone());
        register_source!(
            config.sources.bandcamp,
            "Bandcamp",
            bandcamp_proxy,
            BandcampSource::new(
                config.bandcamp.clone(),
                http_pool.get(bandcamp_proxy.clone())
            )
        );

        let audius_proxy = config.audius.as_ref().and_then(|c| c.proxy.clone());
        register_source!(
            config.sources.audius,
            "Audius",
            audius_proxy,
            AudiusSource::new(config.audius.clone(), http_pool.get(audius_proxy.clone()))
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
        let yandex_proxy = config.yandexmusic.as_ref().and_then(|c| c.proxy.clone());
        register_source!(
            config.sources.yandexmusic && yandex_token_provided,
            "Yandex Music",
            yandex_proxy,
            YandexMusicSource::new(
                config.yandexmusic.clone(),
                http_pool.get(yandex_proxy.clone())
            )
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
            http_pool,
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

            let original_source_name: Option<&str> = self
                .sources
                .iter()
                .find(|s| s.can_handle(identifier))
                .map(|s| s.name());

            let provider_queries: Vec<String> = mirrors
                .providers
                .iter()
                .filter_map(|p| {
                    if isrc.is_empty() && p.contains("%ISRC%") {
                        tracing::debug!("Skipping mirror provider '{}': track has no ISRC", p);
                        return None;
                    }

                    let resolved = p.replace("%ISRC%", isrc).replace("%QUERY%", &query);

                    if let Some(handling_source) = self.sources.iter().find(|s| s.can_handle(&resolved)) {
                        if handling_source.is_mirror() {
                            tracing::warn!(
                                "Skipping mirror provider '{}': '{}' is a Mirror-type source and cannot direct-play",
                                resolved,
                                handling_source.name()
                            );
                            return None;
                        }
                        if Some(handling_source.name()) == original_source_name {
                            tracing::debug!(
                                "Skipping mirror provider '{}': would loop back to the same source '{}'",
                                resolved,
                                handling_source.name()
                            );
                            return None;
                        }
                    }

                    Some(resolved)
                })
                .collect();

            if !provider_queries.is_empty() {
                for sq in provider_queries {
                    let rp = routeplanner.clone();
                    let res = match self.load(&sq, rp.clone()).await {
                        crate::protocol::tracks::LoadResult::Track(t) => {
                            let id = t.info.uri.as_deref().unwrap_or(&t.info.identifier);
                            self.resolve_nested_track(id, rp).await
                        }
                        crate::protocol::tracks::LoadResult::Search(tracks) => {
                            if let Some(first) = tracks.first() {
                                tracing::debug!(
                                    "Mirror provider '{}' returned search result, using first track: {}",
                                    sq,
                                    first.info.identifier
                                );
                                let id =
                                    first.info.uri.as_deref().unwrap_or(&first.info.identifier);
                                self.resolve_nested_track(id, rp).await
                            } else {
                                tracing::debug!(
                                    "Mirror provider '{}' returned empty search result",
                                    sq
                                );
                                None
                            }
                        }
                        _ => {
                            tracing::debug!("Mirror provider '{}' returned no result", sq);
                            None
                        }
                    };

                    if let Some(track) = res {
                        return Some(track);
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
