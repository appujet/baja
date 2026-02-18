use crate::api;
use crate::api::models::*;
use crate::api::tracks::{LoadResult, Track};
use crate::playback::Filters;
use crate::server::AppState;
use crate::sources::SourceManager;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Json},
};
use std::sync::Arc;

/// GET /v4/loadtracks?identifier=...
pub async fn load_tracks(
    Query(params): Query<LoadTracksQuery>,
    State(_state): State<Arc<AppState>>,
) -> Json<LoadResult> {
    let identifier = params.identifier;
    tracing::debug!("Load tracks: '{}'", identifier);

    let source_manager = SourceManager::new();
    Json(source_manager.load(&identifier).await)
}

/// GET /v4/decodetrack?encodedTrack=...
pub async fn decode_track(Query(params): Query<DecodeTrackQuery>) -> impl IntoResponse {
    match Track::decode(&params.encoded_track) {
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
pub async fn decode_tracks(Json(encoded_tracks): Json<Vec<String>>) -> impl IntoResponse {
    let tracks: Vec<Track> = encoded_tracks
        .into_iter()
        .filter_map(|e| Track::decode(&e))
        .collect();
    (StatusCode::OK, Json(tracks)).into_response()
}

/// GET /v4/routeplanner/status
pub async fn routeplanner_status() -> impl IntoResponse {
    // Return 204 No Content if no routeplanner is configured (Lavalink v4 behavior)
    StatusCode::NO_CONTENT
}

/// POST /v4/routeplanner/free/address
pub async fn routeplanner_free_address() -> impl IntoResponse {
    StatusCode::NO_CONTENT
}

/// POST /v4/routeplanner/free/all
pub async fn routeplanner_free_all() -> impl IntoResponse {
    StatusCode::NO_CONTENT
}

/// GET /v4/info
pub async fn get_info() -> Json<api::Info> {
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
        source_managers: SourceManager::new().source_names(),
        filters: Filters::names(),
        plugins: vec![],
    })
}

/// GET /v4/stats
pub async fn get_stats(State(state): State<Arc<AppState>>) -> Json<api::Stats> {
    let uptime = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    Json(crate::monitoring::collect_stats(&state, uptime))
}

/// GET /v4/version
pub async fn get_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}
