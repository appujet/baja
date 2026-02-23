use serde::{Deserialize, Serialize};

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
    #[serde(default = "default_true")]
    pub echo: bool,
    #[serde(default = "default_true")]
    pub high_pass: bool,
    #[serde(default = "default_true")]
    pub normalization: bool,
    #[serde(default = "default_true")]
    pub chorus: bool,
    #[serde(default = "default_true")]
    pub compressor: bool,
    #[serde(default = "default_true")]
    pub flanger: bool,
    #[serde(default = "default_true")]
    pub phaser: bool,
    #[serde(default = "default_true")]
    pub phonograph: bool,
    #[serde(default = "default_true")]
    pub reverb: bool,
    #[serde(default = "default_true")]
    pub spatial: bool,
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
            echo: true,
            high_pass: true,
            normalization: true,
            chorus: true,
            compressor: true,
            flanger: true,
            phaser: true,
            phonograph: true,
            reverb: true,
            spatial: true,
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
            "echo" => self.echo,
            "high_pass" | "highPass" => self.high_pass,
            "normalization" => self.normalization,
            "chorus" => self.chorus,
            "compressor" => self.compressor,
            "flanger" => self.flanger,
            "phaser" => self.phaser,
            "phonograph" => self.phonograph,
            "reverb" => self.reverb,
            "spatial" => self.spatial,
            _ => true,
        }
    }
}
