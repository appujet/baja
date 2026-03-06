use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json},
};

use crate::{protocol, server::AppState};

/// GET /v4/routeplanner/status
pub async fn routeplanner_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    tracing::info!("GET /v4/routeplanner/status");
    match &state.routeplanner {
        Some(rp) => (StatusCode::OK, Json(rp.get_status())).into_response(),
        None => StatusCode::NO_CONTENT.into_response(),
    }
}

pub async fn routeplanner_free_address(
    State(state): State<Arc<AppState>>,
    Json(body): Json<protocol::FreeAddressRequest>,
) -> impl IntoResponse {
    tracing::info!(
        "POST /v4/routeplanner/free/address: address='{}'",
        body.address
    );

    let Some(rp) = &state.routeplanner else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(crate::common::RustalinkError::new(
                500,
                "Internal Server Error",
                "Route planner is disabled",
                "/v4/routeplanner/free/address",
            )),
        )
            .into_response();
    };

    rp.free_address(&body.address);
    StatusCode::NO_CONTENT.into_response()
}

pub async fn routeplanner_free_all(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    tracing::info!("POST /v4/routeplanner/free/all");

    let Some(rp) = &state.routeplanner else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(crate::common::RustalinkError::new(
                500,
                "Internal Server Error",
                "Route planner is disabled",
                "/v4/routeplanner/free/all",
            )),
        )
            .into_response();
    };

    rp.free_all_addresses();
    StatusCode::NO_CONTENT.into_response()
}
