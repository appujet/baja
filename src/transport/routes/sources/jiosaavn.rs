use std::sync::Arc;

use axum::{
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::IntoResponse,
};

use crate::{
    server::AppState,
    sources::jiosaavn::helpers::get_json,
};

pub async fn jiosaavn_stream(
    Path(track_id): Path<String>,
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    tracing::info!("GET /jiosaavn/stream/{}", track_id);

    let ctx = match &state.jiosaavn {
        Some(ctx) => ctx.clone(),
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(serde_json::json!({"error": "JioSaavn source is not enabled"})),
            )
                .into_response();
        }
    };

    let mut params = vec![
        ("__call", "webapi.get"),
        ("api_version", "4"),
        ("_format", "json"),
        ("_marker", "0"),
        ("ctx", "web6dot0"),
        ("token", track_id.as_str()),
        ("type", "song"),
    ];

    let mut track_data = match get_json(&ctx.client, &params).await {
        Some(json) => json.get("songs")
            .and_then(|s| s.get(0))
            .cloned()
            .or_else(|| {
                if json.get("id").is_some() {
                    Some(json)
                } else {
                    None
                }
            }),
        None => None,
    };

    if track_data.is_none() {
        params = vec![
            ("__call", "song.getDetails"),
            ("api_version", "4"),
            ("_format", "json"),
            ("_marker", "0"),
            ("ctx", "web6dot0"),
            ("pids", track_id.as_str()),
        ];
        track_data = match get_json(&ctx.client, &params).await {
            Some(json) => {
                if let Some(arr) = json.as_array() {
                    arr.get(0).cloned()
                } else {
                    json.get("songs")
                        .and_then(|s| s.get(0))
                        .cloned()
                        .or_else(|| if json.get("id").is_some() { Some(json) } else { None })
                }
            }
            None => None,
        };
    }

    let track_data = match track_data {
        Some(d) => d,
        None => {
            return (
                StatusCode::NOT_FOUND,
                axum::Json(serde_json::json!({"error": format!("Track '{}' not found", track_id)})),
            )
                .into_response();
        }
    };

    let encrypted_url = match track_data
        .get("more_info")
        .and_then(|m| m.get("encrypted_media_url"))
        .and_then(|v| v.as_str())
    {
        Some(url) => url,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                axum::Json(serde_json::json!({"error": "No media URL found for track"})),
            )
                .into_response();
        }
    };

    let is_320 = track_data
        .get("more_info")
        .and_then(|m| m.get("320kbps"))
        .map(|v| v.as_str() == Some("true") || v.as_bool() == Some(true))
        .unwrap_or(false);

    let mut playback_url = match ctx.decrypt_url(encrypted_url) {
        Some(url) => url,
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(serde_json::json!({"error": "Failed to decrypt media URL"})),
            )
                .into_response();
        }
    };

    if is_320 {
        playback_url = playback_url.replace("_96.mp4", "_320.mp4");
    }

    let mut request_builder = ctx.client.get(&playback_url);

    if let Some(range) = headers.get(header::RANGE) {
        request_builder = request_builder.header(header::RANGE, range);
    }

    let upstream_res = match request_builder.send().await {
        Ok(res) => res,
        Err(e) => {
            tracing::error!("Failed to proxy JioSaavn request: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(serde_json::json!({"error": format!("Upstream request failed: {}", e)})),
            )
                .into_response();
        }
    };

    let status = upstream_res.status();
    let upstream_headers = upstream_res.headers().clone();

    let mut response_builder = axum::response::Response::builder().status(status);

    if let Some(ct) = upstream_headers.get(header::CONTENT_TYPE) {
        response_builder = response_builder.header(header::CONTENT_TYPE, ct);
    } else {
        let ct = if playback_url.contains(".mp4") { "audio/mp4" } else { "audio/mpeg" };
        response_builder = response_builder.header(header::CONTENT_TYPE, ct);
    }

    if let Some(cl) = upstream_headers.get(header::CONTENT_LENGTH) {
        response_builder = response_builder.header(header::CONTENT_LENGTH, cl);
    }

    if let Some(cr) = upstream_headers.get(header::CONTENT_RANGE) {
        response_builder = response_builder.header(header::CONTENT_RANGE, cr);
    }

    response_builder = response_builder.header(header::ACCEPT_RANGES, "bytes");

    let body = Body::from_stream(upstream_res.bytes_stream());
    response_builder.body(body).unwrap().into_response()
}
