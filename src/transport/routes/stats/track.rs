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
