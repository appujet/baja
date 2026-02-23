use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
#[serde(default)]
pub struct LyricsConfig {
    pub youtube: bool,
    pub lrclib: bool,
    pub genius: bool,
    pub deezer: bool,
    pub bilibili: bool,
    pub musixmatch: bool,
    pub letrasmus: bool,
    pub yandex: bool,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
#[serde(default)]
pub struct YandexLyricsConfig {
    pub access_token: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
#[serde(default)]
pub struct YandexConfig {
    pub lyrics: Option<YandexLyricsConfig>,
}
