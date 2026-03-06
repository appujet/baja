// Core management and traits
pub mod manager;
pub mod plugin;

pub use manager::SourceManager;
pub use plugin::{BoxedSource, BoxedTrack, SourcePlugin};

// Individual source implementations
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
