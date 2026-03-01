use std::sync::Arc;

use axum::{
    Json,
    body::Body,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::IntoResponse,
};
use serde::Deserialize;

use crate::{
    server::AppState,
    sources::youtube::clients::common::{resolve_format_url, select_best_audio_format},
};

pub async fn get_youtube_info(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    tracing::info!("GET /youtube");

    let ctx = match &state.youtube {
        Some(ctx) => ctx.clone(),
        None => {
            tracing::warn!("GET /youtube: YouTube source is not enabled");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "YouTube source is not enabled"})),
            )
                .into_response();
        }
    };

    let tokens = ctx.oauth.get_refresh_tokens().await;
    tracing::debug!("GET /youtube: {} refresh token(s) configured", tokens.len());
    (StatusCode::OK, Json(serde_json::json!({ "refreshTokens": tokens }))).into_response()
}

pub async fn youtube_oauth_refresh(
    Path(refresh_token): Path<String>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    tracing::info!("GET /youtube/oauth/{}", refresh_token);

    let ctx = match &state.youtube {
        Some(ctx) => ctx.clone(),
        None => {
            tracing::warn!("GET /youtube/oauth: YouTube source is not enabled");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "YouTube source is not enabled"})),
            )
                .into_response();
        }
    };

    match ctx.oauth.refresh_with_token(&refresh_token).await {
        Ok(body) => {
            if let Some(err) = body.get("error").and_then(|e| e.as_str()) {
                tracing::warn!(
                    "GET /youtube/oauth/{}: token refresh returned error: {}",
                    refresh_token,
                    err
                );
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": format!("Refreshing access token returned error: {}", err)
                    })),
                )
                    .into_response();
            }
            tracing::debug!("GET /youtube/oauth/{}: token refreshed successfully", refresh_token);
            (StatusCode::OK, Json(body)).into_response()
        }
        Err(e) => {
            tracing::error!("GET /youtube/oauth/{}: {}", refresh_token, e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e.to_string()})),
            )
                .into_response()
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamQuery {
    pub itag: Option<u64>,
    pub with_client: Option<String>,
}

pub async fn youtube_stream(
    Path(video_id): Path<String>,
    Query(params): Query<StreamQuery>,
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    tracing::info!(
        "GET /youtube/stream/{} itag={:?} withClient={:?} Range={:?}",
        video_id,
        params.itag,
        params.with_client,
        headers.get(header::RANGE)
    );

    let ctx = match &state.youtube {
        Some(ctx) => ctx.clone(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "YouTube source is not enabled"})),
            )
                .into_response();
        }
    };

    let clients: Vec<_> = if let Some(ref filter) = params.with_client {
        if let Some(client) = ctx.get_playback_client_by_name(filter) {
            vec![client]
        } else {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": format!("No client named '{}' is configured for playback", filter)
                })),
            )
                .into_response();
        }
    } else {
        ctx.clients.iter().cloned().collect()
    };

    let visitor_data_str: Option<String> = ctx.visitor_data.read().await.clone();
    let visitor_data = visitor_data_str.as_deref();
    let player_page_url = format!("https://www.youtube.com/watch?v={}", video_id);

    let signature_timestamp = ctx.cipher_manager.get_signature_timestamp().await.ok();

    let mut last_error: Option<String> = None;
    let mut found_formats = false;
    let mut last_was_exception = false;

    for client in &clients {
        // Skip WEB client as requested
        if client.client_name().to_uppercase() == "WEB" {
            tracing::debug!("GET /youtube/stream/{}: skipping WEB client", video_id);
            continue;
        }

        tracing::debug!(
            "GET /youtube/stream/{}: attempting client '{}'",
            video_id,
            client.name()
        );

        let body = match client
            .get_player_body(
                &video_id,
                visitor_data,
                signature_timestamp,
                ctx.oauth.clone(),
            )
            .await
        {
            Some(b) => b,
            None => {
                tracing::debug!(
                    "GET /youtube/stream/{}: client '{}' failed to get player body, skipping",
                    video_id,
                    client.name()
                );
                continue;
            }
        };

        let playability = body
            .get("playabilityStatus")
            .and_then(|p| p.get("status"))
            .and_then(|s| s.as_str())
            .unwrap_or("UNKNOWN");

        if playability != "OK" {
            let reason = body
                .get("playabilityStatus")
                .and_then(|p| p.get("reason"))
                .and_then(|r| r.as_str())
                .unwrap_or("no reason provided");
            last_error = Some(format!(
                "Video '{}' is not playable via client '{}': status={}, reason={}",
                video_id,
                client.name(),
                playability,
                reason
            ));
            tracing::warn!("{}", last_error.as_deref().unwrap());
            continue;
        }

        let streaming_data = match body.get("streamingData") {
            Some(sd) => sd,
            None => {
                last_error = Some(format!(
                    "Client '{}' returned no streamingData for video '{}'",
                    client.name(),
                    video_id
                ));
                tracing::warn!("{}", last_error.as_deref().unwrap());
                continue;
            }
        };

        let adaptive = streaming_data
            .get("adaptiveFormats")
            .and_then(|v| v.as_array());
        let formats = streaming_data.get("formats").and_then(|v| v.as_array());

        found_formats = true;
        last_was_exception = false;

        let format = if let Some(target_itag) = params.itag {
            let all_formats: Vec<&serde_json::Value> = adaptive
                .iter()
                .flat_map(|v| v.iter())
                .chain(formats.iter().flat_map(|v| v.iter()))
                .collect();

            tracing::debug!(
                "GET /youtube/stream/{}: client '{}' searching for itag {}",
                video_id,
                client.name(),
                target_itag
            );

            let found = all_formats.into_iter().find(|f| {
                f.get("itag")
                    .and_then(|v| v.as_u64())
                    .map(|i| i == target_itag)
                    .unwrap_or(false)
            });

            match found {
                Some(f) => f,
                None => {
                    last_error = Some(format!(
                        "itag {} not found in formats returned by client '{}' for video '{}'",
                        target_itag,
                        client.name(),
                        video_id
                    ));
                    tracing::debug!("{}", last_error.as_deref().unwrap());
                    continue;
                }
            }
        } else {
            tracing::debug!(
                "GET /youtube/stream/{}: client '{}' selecting best audio format (opus preferred)",
                video_id,
                client.name()
            );
            match select_best_audio_format(adaptive, formats) {
                Some(f) => f,
                None => {
                    last_error = Some(format!(
                        "Client '{}' returned no suitable audio formats for video '{}'",
                        client.name(),
                        video_id
                    ));
                    tracing::warn!("{}", last_error.as_deref().unwrap());
                    continue;
                }
            }
        };

        let selected_itag = format.get("itag").and_then(|v| v.as_i64());
        tracing::debug!(
            "GET /youtube/stream/{}: client '{}' selected itag={:?}, resolving URL",
            video_id,
            client.name(),
            selected_itag
        );

        let resolved_url =
            match resolve_format_url(format, &player_page_url, &ctx.cipher_manager).await {
                Ok(Some(url)) => url,
                Ok(None) => {
                    last_error = Some(format!(
                        "Client '{}' could not resolve a URL for video '{}' itag={:?}",
                        client.name(),
                        video_id,
                        selected_itag
                    ));
                    last_was_exception = true;
                    tracing::warn!("{}", last_error.as_deref().unwrap());
                    continue;
                }
                Err(e) => {
                    last_error = Some(format!(
                        "Client '{}' cipher/n-param resolution failed for video '{}': {}",
                        client.name(),
                        video_id,
                        e
                    ));
                    last_was_exception = true;
                    tracing::error!("{}", last_error.as_deref().unwrap());
                    continue;
                }
            };

        let content_length = format
            .get("contentLength")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);

        let mime_type = format
            .get("mimeType")
            .and_then(|v| v.as_str())
            .unwrap_or("application/octet-stream")
            .to_string();

        let mut final_url: reqwest::Url = match reqwest::Url::parse(&resolved_url) {
            Ok(u) => u,
            Err(e) => {
                last_error = Some(format!(
                    "Failed to parse resolved URL for video '{}': {}",
                    video_id, e
                ));
                last_was_exception = true;
                tracing::error!("{}", last_error.as_deref().unwrap());
                continue;
            }
        };

        let (start, end) = if let Some(range_val) = headers.get(header::RANGE).and_then(|v| v.to_str().ok()) {
            if let Some(caps) = regex::Regex::new(r"bytes=(\d+)-(\d+)?")
                .unwrap()
                .captures(range_val)
            {
                let s = caps.get(1).map(|m| m.as_str().parse::<u64>().unwrap_or(0)).unwrap_or(0);
                let e = caps.get(2).map(|m| m.as_str().parse::<u64>().unwrap_or(content_length.saturating_sub(1))).unwrap_or(content_length.saturating_sub(1));
                (s, e)
            } else {
                (0, content_length.saturating_sub(1))
            }
        } else {
            (0, content_length.saturating_sub(1))
        };

        final_url
            .query_pairs_mut()
            .append_pair("range", &format!("{}-{}", start, end));

        let bitrate = format.get("bitrate").and_then(|v| v.as_i64()).unwrap_or(0);

        tracing::info!(
            "GET /youtube/stream/{}: streaming via client '{}' itag={:?} bitrate={} bps mime={} range={}-{}",
            video_id,
            client.name(),
            selected_itag,
            bitrate,
            mime_type,
            start,
            end
        );

        let upstream = match ctx.http.get(final_url.as_str()).send().await {
            Ok(r) => r,
            Err(e) => {
                last_error = Some(format!(
                    "Upstream request failed for client '{}': {}",
                    client.name(),
                    e
                ));
                last_was_exception = true;
                tracing::error!("{}", last_error.as_deref().unwrap());
                continue;
            }
        };

        if !upstream.status().is_success() {
            last_error = Some(format!(
                "Upstream returned {} for client '{}' for video '{}'",
                upstream.status(),
                client.name(),
                video_id
            ));
            last_was_exception = true;
            tracing::warn!("{}", last_error.as_deref().unwrap());
            // If 403 or other non-success, try next client
            continue;
        }

        let mut resp_headers = HeaderMap::new();
        if let Ok(v) = header::HeaderValue::from_str(&mime_type) {
            resp_headers.insert(header::CONTENT_TYPE, v);
        }

        if let Some(v) = upstream.headers().get(header::CONTENT_LENGTH) {
            resp_headers.insert(header::CONTENT_LENGTH, v.clone());
        }
        if let Some(v) = upstream.headers().get(header::CONTENT_RANGE) {
            resp_headers.insert(header::CONTENT_RANGE, v.clone());
        } else if start > 0 || end < content_length.saturating_sub(1) {
            let cr = format!("bytes {}-{}/{}", start, end, content_length);
            if let Ok(v) = header::HeaderValue::from_str(&cr) {
                resp_headers.insert(header::CONTENT_RANGE, v);
            }
        }

        return (
            upstream.status(),
            resp_headers,
            Body::from_stream(upstream.bytes_stream()),
        )
            .into_response();
    }

    let (status, error_msg) = if found_formats && params.itag.is_some() && !last_was_exception {
        (
            StatusCode::BAD_REQUEST,
            last_error.unwrap_or_else(|| {
                format!("No formats found with the requested itag for video '{}'", video_id)
            }),
        )
    } else if last_was_exception {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            last_error.unwrap_or_else(|| format!("This video cannot be loaded: '{}'", video_id)),
        )
    } else {
        (
            StatusCode::BAD_REQUEST,
            last_error
                .unwrap_or_else(|| format!("Could not find formats for video '{}'", video_id)),
        )
    };

    tracing::warn!("GET /youtube/stream/{}: {}", video_id, error_msg);
    (status, Json(serde_json::json!({ "error": error_msg }))).into_response()
}
