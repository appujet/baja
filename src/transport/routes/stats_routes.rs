use std::sync::Arc;

use axum::{
  extract::{Query, State},
  http::StatusCode,
  response::{IntoResponse, Json},
};

use crate::{
  api,
  api::{
    models::*,
    tracks::{LoadResult, Track},
  },
  player::Filters,
  server::AppState,
};

/// GET /v4/loadtracks?identifier=...
pub async fn load_tracks(
  Query(params): Query<LoadTracksQuery>,
  State(state): State<Arc<AppState>>,
) -> Json<LoadResult> {
  let identifier = params.identifier;
  tracing::info!("GET /v4/loadtracks: identifier='{}'", identifier);

  Json(
    state
      .source_manager
      .load(&identifier, state.routeplanner.clone())
      .await,
  )
}

/// GET /v4/decodetrack?encodedTrack=...
pub async fn decode_track(Query(params): Query<DecodeTrackQuery>) -> impl IntoResponse {
  tracing::info!("GET /v4/decodetrack");
  let encoded = params.encoded_track.or(params.track);

  let encoded = match encoded {
    Some(e) => e,
    None => {
      return (
        StatusCode::BAD_REQUEST,
        Json(crate::common::LavalinkError::bad_request(
          "No track to decode provided",
          "/v4/decodetrack",
        )),
      )
        .into_response();
    }
  };

  match Track::decode(&encoded) {
    Some(track) => (StatusCode::OK, Json(track)).into_response(),
    None => (
      StatusCode::BAD_REQUEST,
      Json(crate::common::LavalinkError::bad_request(
        "Invalid track encoding",
        "/v4/decodetrack",
      )),
    )
      .into_response(),
  }
}

/// POST /v4/decodetracks
pub async fn decode_tracks(Json(body): Json<api::EncodedTracks>) -> impl IntoResponse {
  tracing::info!("POST /v4/decodetracks: count={}", body.tracks.len());
  let tracks: Vec<Track> = body
    .tracks
    .into_iter()
    .filter_map(|e| Track::decode(&e))
    .collect();
  (StatusCode::OK, Json(tracks)).into_response()
}

/// GET /v4/routeplanner/status
pub async fn routeplanner_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
  tracing::info!("GET /v4/routeplanner/status");
  match &state.routeplanner {
    Some(rp) => (StatusCode::OK, Json(rp.get_status())).into_response(),
    None => StatusCode::NO_CONTENT.into_response(),
  }
}

/// POST /v4/routeplanner/free/address
pub async fn routeplanner_free_address(
  State(state): State<Arc<AppState>>,
  Json(body): Json<api::FreeAddressRequest>,
) -> impl IntoResponse {
  tracing::info!(
    "POST /v4/routeplanner/free/address: address='{}'",
    body.address
  );
  if let Some(rp) = &state.routeplanner {
    rp.free_address(&body.address);
  }
  StatusCode::NO_CONTENT
}

/// POST /v4/routeplanner/free/all
pub async fn routeplanner_free_all(State(state): State<Arc<AppState>>) -> impl IntoResponse {
  tracing::info!("POST /v4/routeplanner/free/all");
  if let Some(rp) = &state.routeplanner {
    rp.free_all_addresses();
  }
  StatusCode::NO_CONTENT
}

/// GET /v4/info
pub async fn get_info(State(state): State<Arc<AppState>>) -> Json<api::Info> {
  tracing::info!("GET /v4/info");
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
  tracing::info!("GET /v4/stats");
  let uptime = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .unwrap_or_default()
    .as_millis() as u64;
  Json(crate::monitoring::collect_stats(&state, uptime))
}

/// GET /version
pub async fn get_version() -> String {
  tracing::info!("GET /version");
  env!("CARGO_PKG_VERSION").to_string()
}
