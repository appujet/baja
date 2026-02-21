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
      _ => true,
    }
  }
}
