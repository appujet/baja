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

/// GET /v4/loadsearch?query=...&types=...
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
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string())
        .filter(|s| match s.as_str() {
            "track" | "album" | "artist" | "playlist" | "text" => true,
            _ => false,
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

/// GET /v4/decodetrack?encodedTrack=...
pub async fn decode_track(Query(params): Query<DecodeTrackQuery>) -> impl IntoResponse {
    let encoded = params.encoded_track.clone().or(params.track.clone());
    tracing::info!("GET /v4/decodetrack: encodedTrack={:?}", encoded);

    let encoded = match encoded {
        Some(e) => e,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(crate::common::RustalinkError::bad_request(
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
            Json(crate::common::RustalinkError::bad_request(
                "Invalid track encoding",
                "/v4/decodetrack",
            )),
        )
            .into_response(),
    }
}

/// POST /v4/decodetracks
/// Body: raw JSON array of base64-encoded track strings e.g. ["enc1","enc2"]
pub async fn decode_tracks(Json(body): Json<protocol::EncodedTracks>) -> impl IntoResponse {
    let tracks_input = body.0;
    tracing::info!("POST /v4/decodetracks: count={}", tracks_input.len());

    if tracks_input.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(
                serde_json::to_value(crate::common::RustalinkError::bad_request(
                    "No tracks to decode provided",
                    "/v4/decodetracks",
                ))
                .unwrap(),
            ),
        )
            .into_response();
    }

    let mut tracks = Vec::with_capacity(tracks_input.len());
    for encoded in &tracks_input {
        match Track::decode(encoded) {
            Some(t) => tracks.push(t),
            None => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(
                        serde_json::to_value(crate::common::RustalinkError::bad_request(
                            format!("Invalid track encoding: {}", encoded),
                            "/v4/decodetracks",
                        ))
                        .unwrap(),
                    ),
                )
                    .into_response();
            }
        }
    }

    (StatusCode::OK, Json(tracks)).into_response()
}
