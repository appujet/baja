use serde::Serialize;

/// Server statistics.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
  pub players: i32,
  pub playing_players: i32,
  pub uptime: u64,
  pub memory: Memory,
  pub cpu: Cpu,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub frame_stats: Option<FrameStats>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Memory {
  pub free: u64,
  pub used: u64,
  pub allocated: u64,
  pub reservable: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Cpu {
  pub cores: i32,
  pub system_load: f64,
  pub lavalink_load: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameStats {
  pub sent: i32,
  pub nulled: i32,
  pub deficit: i32,
}
