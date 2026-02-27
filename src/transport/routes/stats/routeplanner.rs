use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json},
};

use crate::{api, server::AppState};

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
    match &state.routeplanner {
    Some(rp) => {
      rp.free_address(&body.address);
      StatusCode::NO_CONTENT.into_response()
    }
    None => (
      StatusCode::INTERNAL_SERVER_ERROR,
      Json(serde_json::json!({ "message": "Can't access disabled route planner", "status": 500 })),
    )
      .into_response(),
  }
}

/// POST /v4/routeplanner/free/all
pub async fn routeplanner_free_all(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    tracing::info!("POST /v4/routeplanner/free/all");
    match &state.routeplanner {
    Some(rp) => {
      rp.free_all_addresses();
      StatusCode::NO_CONTENT.into_response()
    }
    None => (
      StatusCode::INTERNAL_SERVER_ERROR,
      Json(serde_json::json!({ "message": "Can't access disabled route planner", "status": 500 })),
    )
      .into_response(),
  }
}
