use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Config {
    pub server: ServerConfig,
    pub route_planner: RoutePlannerConfig,
    pub sources: SourcesConfig,
    pub logging: Option<LoggingConfig>,
    #[serde(default)]
    pub filters: FiltersConfig,
    #[serde(default)]
    pub jiosaavn: Option<JioSaavnConfig>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct FiltersConfig {
    #[serde(default = "default_true")]
    pub volume: bool,
    #[serde(default = "default_true")]
    pub equalizer: bool,
    #[serde(default = "default_true")]
    pub karaoke: bool,
    #[serde(default = "default_true")]
    pub timescale: bool,
    #[serde(default = "default_true")]
    pub tremolo: bool,
    #[serde(default = "default_true")]
    pub vibrato: bool,
    #[serde(default = "default_true")]
    pub distortion: bool,
    #[serde(default = "default_true")]
    pub rotation: bool,
    #[serde(default = "default_true")]
    pub channel_mix: bool,
    #[serde(default = "default_true")]
    pub low_pass: bool,
}

fn default_true() -> bool {
    true
}

impl Default for FiltersConfig {
    fn default() -> Self {
        Self {
            volume: true,
            equalizer: true,
            karaoke: true,
            timescale: true,
            tremolo: true,
            vibrato: true,
            distortion: true,
            rotation: true,
            channel_mix: true,
            low_pass: true,
        }
    }
}

impl FiltersConfig {
    pub fn is_enabled(&self, name: &str) -> bool {
        match name {
            "volume" => self.volume,
            "equalizer" => self.equalizer,
            "karaoke" => self.karaoke,
            "timescale" => self.timescale,
            "tremolo" => self.tremolo,
            "vibrato" => self.vibrato,
            "distortion" => self.distortion,
            "rotation" => self.rotation,
            "channel_mix" | "channelMix" => self.channel_mix,
            "low_pass" | "lowPass" => self.low_pass,
            _ => true, // Default to true for unknown filters (e.g. plugins) to avoid breaking changes
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub password: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct RoutePlannerConfig {
    pub enabled: bool,
    pub cidrs: Vec<String>,
    pub excluded_ips: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct SourcesConfig {
    pub youtube: bool,
    pub spotify: bool,
    pub http: bool,
    pub local: bool,
    #[serde(default)]
    pub jiosaavn: bool,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct LoggingConfig {
    pub level: Option<String>,
    pub filters: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct JioSaavnConfig {
    pub decryption: Option<JioSaavnDecryptionConfig>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct JioSaavnDecryptionConfig {
    #[serde(rename = "secretKey")]
    pub secret_key: Option<String>,
}

impl Config {
    pub fn load() -> Result<Self, Box<dyn std::error::Error>> {
        let config_str = std::fs::read_to_string("config.toml").unwrap_or_else(|_| "".to_string());
        if config_str.is_empty() {
            return Err("config.toml not found or empty".into());
        }
        let config: Config = toml::from_str(&config_str)?;
        Ok(config)
    }
}
