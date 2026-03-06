use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Json},
};

use crate::{
    protocol,
    protocol::{
        models::*,
        tracks::{LoadResult, Track},
    },
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

pub async fn load_search(
    Query(params): Query<LoadSearchQuery>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let query = params.query;
    let types_str = params.types.unwrap_or_default();

    tracing::info!(
        "GET /v4/loadsearch: query='{}', types='{}'",
        query,
        types_str
    );

    let types: Vec<String> = types_str
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .filter(|s| {
            matches!(
                s.as_str(),
                "track" | "album" | "artist" | "playlist" | "text"
            )
        })
        .collect();

    match state
        .source_manager
        .load_search(&query, &types, state.routeplanner.clone())
        .await
    {
        Some(result) => (StatusCode::OK, Json(result)).into_response(),
        None => StatusCode::NO_CONTENT.into_response(),
    }
}

pub async fn decode_track(Query(params): Query<DecodeTrackQuery>) -> impl IntoResponse {
    let encoded = params.encoded_track.clone().or(params.track);
    tracing::info!("GET /v4/decodetrack: encodedTrack={:?}", encoded);

    let Some(encoded) = encoded else {
        return (
            StatusCode::BAD_REQUEST,
            Json(crate::common::RustalinkError::bad_request(
                "No track to decode provided",
                "/v4/decodetrack",
            )),
        )
            .into_response();
    };

    match Track::decode(&encoded) {
        Some(track) => (StatusCode::OK, Json(track)).into_response(),
        None => (
            StatusCode::BAD_REQUEST,
            Json(crate::common::RustalinkError::bad_request(
                "Invalid track encoding",
                "/v4/decodetrack",
            )),
        )
            .into_response(),
    }
}

pub async fn decode_tracks(Json(body): Json<protocol::EncodedTracks>) -> impl IntoResponse {
    let tracks_input = body.0;
    tracing::info!("POST /v4/decodetracks: count={}", tracks_input.len());

    if tracks_input.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(crate::common::RustalinkError::bad_request(
                "No tracks to decode provided",
                "/v4/decodetracks",
            )),
        )
            .into_response();
    }

    let mut tracks = Vec::with_capacity(tracks_input.len());
    for encoded in &tracks_input {
        let Some(t) = Track::decode(encoded) else {
            return (
                StatusCode::BAD_REQUEST,
                Json(crate::common::RustalinkError::bad_request(
                    format!("Invalid track encoding: {}", encoded),
                    "/v4/decodetracks",
                )),
            )
                .into_response();
        };
        tracks.push(t);
    }

    (StatusCode::OK, Json(tracks)).into_response()
}
