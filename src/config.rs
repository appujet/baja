use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Config {
    pub server: ServerConfig,
    pub route_planner: RoutePlannerConfig,
    pub sources: SourcesConfig,
    pub logging: Option<LoggingConfig>,
    #[serde(default)]
    pub filters: FiltersConfig,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct FiltersConfig {
    #[serde(default)]
    pub enabled: EnabledFiltersConfig,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct EnabledFiltersConfig {
    pub volume: bool,
    pub equalizer: bool,
    pub karaoke: bool,
    pub timescale: bool,
    pub tremolo: bool,
    pub vibrato: bool,
    pub distortion: bool,
    pub rotation: bool,
    pub channel_mix: bool,
    pub low_pass: bool,
}

impl Default for EnabledFiltersConfig {
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

impl EnabledFiltersConfig {
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
            "channel_mix" => self.channel_mix,
            "low_pass" => self.low_pass,
            _ => true, // Unknown filters are allowed by default or should be handled by plugin logic?
                       // For now, strict validation only for known core filters.
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
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct LoggingConfig {
    pub level: Option<String>,
    pub filters: Option<String>,
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
