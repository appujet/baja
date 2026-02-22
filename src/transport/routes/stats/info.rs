use std::sync::Arc;

use axum::{extract::State, response::Json};

use crate::{api, player::Filters, server::AppState};

/// GET /v4/info
pub async fn get_info(State(state): State<Arc<AppState>>) -> Json<api::Info> {
  tracing::debug!("GET /v4/info");
  let version_str = env!("CARGO_PKG_VERSION");
  let mut parts = version_str.split('.');
  let major = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
  let minor = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
  let patch = parts
    .next()
    .and_then(|s| {
      s.split('-')
        .next()
        .and_then(|s| s.split('+').next())
        .and_then(|s| s.parse().ok())
    })
    .unwrap_or(0);

  Json(api::Info {
    version: api::Version {
      semver: version_str.to_string(),
      major,
      minor,
      patch,
      pre_release: None,
      build: None,
    },
    build_time: option_env!("BUILD_TIME")
      .and_then(|s| s.parse().ok())
      .unwrap_or(0),
    git: api::GitInfo {
      branch: option_env!("GIT_BRANCH").unwrap_or("unknown").to_string(),
      commit: option_env!("GIT_COMMIT").unwrap_or("unknown").to_string(),
      commit_time: option_env!("GIT_COMMIT_TIME")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0),
    },
    jvm: "Rust".to_string(),
    lavaplayer: "symphonia".to_string(),
    source_managers: state.source_manager.source_names(),
    filters: Filters::names()
      .into_iter()
      .filter(|name| state.config.filters.is_enabled(name))
      .collect(),
    plugins: vec![],
  })
}

/// GET /v4/stats
pub async fn get_stats(State(state): State<Arc<AppState>>) -> Json<api::Stats> {
  tracing::debug!("GET /v4/stats");
  let uptime = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .unwrap_or_default()
    .as_millis() as u64;
  Json(crate::monitoring::collect_stats(&state, uptime))
}

/// GET /version
pub async fn get_version() -> String {
  tracing::debug!("GET /version");
  env!("CARGO_PKG_VERSION").to_string()
}
