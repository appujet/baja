use crate::api::models::*;
use crate::server::AppState;
use crate::sources::SourceManager;
use crate::api;
use crate::api::tracks::LoadResult;
use axum::{
    extract::{Query, State},
    response::Json,
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

/// GET /v4/info
pub async fn get_info() -> Json<api::Info> {
    Json(api::Info {
        version: api::Version {
            semver: "4.0.0".to_string(),
            major: 4,
            minor: 0,
            patch: 0,
            pre_release: None,
            build: None,
        },
        build_time: 0,
        git: api::GitInfo {
            branch: "main".to_string(),
            commit: "unknown".to_string(),
            commit_time: 0,
        },
        jvm: "Rust".to_string(),
        lavaplayer: "symphonia".to_string(),
        source_managers: vec!["http".to_string(), "youtube".to_string()],
        filters: vec![
            "volume".into(),
            "equalizer".into(),
            "karaoke".into(),
            "timescale".into(),
            "tremolo".into(),
            "vibrato".into(),
            "distortion".into(),
            "rotation".into(),
            "channelMix".into(),
            "lowPass".into(),
        ],
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
    "4.0.0".to_string()
}
