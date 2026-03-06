use serde::{Deserialize, Serialize};

pub mod anghami;
pub mod applemusic;
pub mod audiomack;
pub mod audius;
pub mod bandcamp;
pub mod deezer;
pub mod flowery;
pub mod gaana;
pub mod google_tts;
pub mod http;
pub mod jiosaavn;
pub mod local;
pub mod mixcloud;
pub mod pandora;
pub mod qobuz;
pub mod shazam;
pub mod soundcloud;
pub mod spotify;
pub mod tidal;
pub mod yandexmusic;
pub mod youtube;

pub use anghami::*;
pub use applemusic::*;
pub use audiomack::*;
pub use audius::*;
pub use bandcamp::*;
pub use deezer::*;
pub use flowery::*;
pub use gaana::*;
pub use google_tts::*;
pub use http::*;
pub use jiosaavn::*;
pub use local::*;
pub use mixcloud::*;
pub use pandora::*;
pub use qobuz::*;
pub use shazam::*;
pub use soundcloud::*;
pub use spotify::*;
pub use tidal::*;
pub use yandexmusic::*;
pub use youtube::*;

#[derive(Debug, Deserialize, Serialize, Clone, Default, PartialEq, Eq, Hash)]
pub struct HttpProxyConfig {
    pub url: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
#[serde(default)]
pub struct SourcesConfig {
    pub youtube: Option<YouTubeConfig>,
    pub spotify: Option<SpotifyConfig>,
    pub http: Option<HttpSourceConfig>,
    pub local: Option<LocalSourceConfig>,
    pub jiosaavn: Option<JioSaavnConfig>,
    pub deezer: Option<DeezerConfig>,
    pub applemusic: Option<AppleMusicConfig>,
    pub gaana: Option<GaanaConfig>,
    pub tidal: Option<TidalConfig>,
    pub soundcloud: Option<SoundCloudConfig>,
    pub audiomack: Option<AudiomackConfig>,
    pub audius: Option<AudiusConfig>,
    pub pandora: Option<PandoraConfig>,
    pub qobuz: Option<QobuzConfig>,
    pub anghami: Option<AnghamiConfig>,
    pub shazam: Option<ShazamConfig>,
    pub mixcloud: Option<MixcloudConfig>,
    pub bandcamp: Option<BandcampConfig>,
    pub yandexmusic: Option<YandexMusicConfig>,
    pub google_tts: Option<GoogleTtsConfig>,
    pub flowery: Option<FloweryConfig>,
}

pub fn default_true() -> bool {
    true
}
pub fn default_false() -> bool {
    false
}

pub fn default_limit_10() -> usize {
    10
}
pub fn default_limit_20() -> usize {
    20
}
pub fn default_limit_50() -> usize {
    50
}
pub fn default_limit_100() -> usize {
    100
}
pub fn default_limit_3000() -> usize {
    3000
}

pub fn default_country_code() -> String {
    "us".to_string()
}
pub fn default_zero() -> usize {
    0
}
pub fn default_five() -> usize {
    5
}
