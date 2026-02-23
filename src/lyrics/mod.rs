use async_trait::async_trait;
use std::sync::Arc;
use futures::stream::{FuturesUnordered, StreamExt};
use crate::{api::models::LyricsData, api::tracks::TrackInfo, configs::Config};

pub mod lrclib;
pub mod genius;
pub mod youtube;
pub mod deezer;
pub mod bilibili;
pub mod musixmatch;
pub mod letrasmus;
pub mod yandex;
pub mod netease;

use self::genius::GeniusProvider;
use self::lrclib::LrcLibProvider;
use self::youtube::YoutubeLyricsProvider;
use self::deezer::DeezerProvider;
use self::bilibili::BilibiliProvider;
use self::musixmatch::MusixmatchProvider;
use letrasmus::LetrasMusProvider;
use yandex::YandexProvider;
use netease::NeteaseProvider;

#[async_trait]
pub trait LyricsProvider: Send + Sync {
    fn name(&self) -> &'static str;
    async fn load_lyrics(
        &self,
        track: &TrackInfo,
        language: Option<String>,
        source_manager: Option<Arc<crate::sources::SourceManager>>,
    ) -> Option<LyricsData>;
}

pub struct LyricsManager {
    pub providers: Vec<Arc<dyn LyricsProvider>>,
    pub source_manager: Option<Arc<crate::sources::SourceManager>>,
}

impl LyricsManager {
    pub fn new(config: &Config) -> Self {
        let mut providers: Vec<Arc<dyn LyricsProvider>> = Vec::new();

        macro_rules! register_provider {
            ($enabled:expr, $name:literal, $ctor:expr) => {
                if $enabled {
                    providers.push(Arc::new($ctor));
                    tracing::info!("Loaded lyrics provider: {}", $name);
                }
            };
        }

        register_provider!(config.lyrics.youtube, "YouTube", YoutubeLyricsProvider::new());
        register_provider!(config.lyrics.lrclib, "LRCLib", LrcLibProvider::new());
        register_provider!(config.lyrics.genius, "Genius", GeniusProvider::new());
        register_provider!(config.lyrics.deezer, "Deezer", DeezerProvider::new());
        register_provider!(config.lyrics.bilibili, "Bilibili", BilibiliProvider::new());
        register_provider!(config.lyrics.musixmatch, "Musixmatch", MusixmatchProvider::new());
        register_provider!(config.lyrics.letrasmus, "Letras.mus", LetrasMusProvider::new());
        register_provider!(config.lyrics.netease, "NetEase", NeteaseProvider::new());
        let yandex_token_provided = config
            .yandex
            .as_ref()
            .and_then(|y| y.lyrics.as_ref())
            .and_then(|l| l.access_token.as_ref())
            .map(|t| !t.is_empty())
            .unwrap_or(false);

        if config.lyrics.yandex && !yandex_token_provided {
            tracing::warn!("Yandex lyrics enabled but access_token is missing; it will be disabled.");
        }

        if let Some(yandex_lyrics_cfg) = config.yandex.as_ref().and_then(|y| y.lyrics.as_ref()) {
            register_provider!(
                config.lyrics.yandex && yandex_token_provided,
                "Yandex Music",
                YandexProvider::new(yandex_lyrics_cfg)
            );
        }

        Self {
            providers,
            source_manager: None,
        }
    }

    pub fn set_source_manager(&mut self, source_manager: Arc<crate::sources::SourceManager>) {
        self.source_manager = Some(source_manager);
    }

    pub async fn load_lyrics(&self, track: &TrackInfo, language: Option<String>) -> Option<LyricsData> {
        self.load_lyrics_ext(track, language, false).await
    }

    pub async fn load_lyrics_ext(
        &self,
        track: &TrackInfo,
        language: Option<String>,
        skip_track_source: bool,
    ) -> Option<LyricsData> {
        let mut futures = FuturesUnordered::new();

        for provider in &self.providers {
            if skip_track_source && provider.name().eq_ignore_ascii_case(&track.source_name) {
                continue;
            }
            let provider = provider.clone();
            let track = track.clone();
            let language = language.clone();
            let source_manager = self.source_manager.clone();
            futures.push(async move {
                provider.load_lyrics(&track, language, source_manager).await
            });
        }

        let mut fallback_text: Option<LyricsData> = None;

        while let Some(result) = futures.next().await {
            if let Some(lyrics) = result {
                if lyrics.lines.is_some() {
                    // Return synced lyrics immediately
                    return Some(lyrics);
                } else if fallback_text.is_none() {
                    // Store the first plain text result as fallback
                    fallback_text = Some(lyrics);
                }
            }
        }

        fallback_text
    }
}
