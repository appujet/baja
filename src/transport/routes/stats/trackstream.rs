use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Json, Response},
    body::Body,
};
use futures::StreamExt;

use crate::{
    common::RustalinkError,
    protocol::{
        models::TrackStreamQuery,
        tracks::Track,
    },
    sources::deezer::reader::crypt::{DeezerCrypt, CHUNK_SIZE},
    server::AppState,
};

const PATH: &str = "/v4/trackstream";

/// GET /v4/trackstream?encodedTrack=...&videoId=...&itag=...&withClient=...
pub async fn track_stream(
    headers: HeaderMap,
    Query(params): Query<TrackStreamQuery>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    tracing::info!(
        "GET /v4/trackstream: encodedTrack={:?} videoId={:?} itag={:?} withClient={:?} (Range: {:?})",
        params.encoded_track,
        params.video_id,
        params.itag,
        params.with_client,
        headers.get(header::RANGE)
    );

    if params.encoded_track.is_none() && params.video_id.is_none() {
        return (
            StatusCode::BAD_REQUEST,
            Json(RustalinkError::bad_request(
                "Missing required query parameter: either 'encodedTrack' or 'videoId' must be provided",
                PATH,
            )),
        )
            .into_response();
    }

    // encodedTrack takes priority; videoId is only used when encodedTrack is absent
    if params.encoded_track.is_none() {
        let video_id = params.video_id.as_deref().unwrap();
        return stream_youtube(video_id, params.itag, params.with_client.as_deref(), headers, &state).await;
    }

    let encoded = params.encoded_track.unwrap().replace(' ', "+");
    let track = match Track::decode(&encoded) {
        Some(t) => t,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(RustalinkError::bad_request(
                    "Invalid track encoding: failed to decode encodedTrack",
                    PATH,
                )),
            )
                .into_response();
        }
    };

    let source_name = &track.info.source_name;
    let identifier = track.info.uri.as_deref().unwrap_or(&track.info.identifier);

    let source = match state
        .source_manager
        .sources
        .iter()
        .find(|s| s.name() == source_name.as_str())
    {
        Some(s) => s,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(RustalinkError::not_found(
                    format!("Source '{}' is not registered or disabled", source_name),
                    PATH,
                )),
            )
                .into_response();
        }
    };

    if source.is_mirror() {
        return (
            StatusCode::BAD_REQUEST,
            Json(RustalinkError::bad_request(
                format!(
                    "Source '{}' is a mirror-only source and does not support direct streaming",
                    source_name
                ),
                PATH,
            )),
        )
            .into_response();
    }

    // itag and withClient are YouTube-specific; forward them only for YouTube sources
    if matches!(source_name.as_str(), "youtube" | "youtubemusic") {
        return stream_youtube(
            &track.info.identifier,
            params.itag,
            params.with_client.as_deref(),
            headers,
            &state,
        )
        .await;
    }

    match source.get_stream_url(identifier, params.itag).await {
        Some(info) => {
            if let Some(rest) = info.url.strip_prefix("deezer_encrypted:") {
                let mut parts = rest.splitn(2, ':');
                let track_id = parts.next().unwrap_or("").to_string();
                let final_url = parts.next().unwrap_or("").to_string();

                let master_key = if let Some(dz) = &state.source_manager.deezer {
                    dz.token_tracker.get_token().await
                        .map(|_| state.config.deezer.as_ref()
                            .and_then(|c| c.master_decryption_key.clone())
                            .unwrap_or_default())
                        .unwrap_or_default()
                } else {
                    String::new()
                };

                return proxy_stream(
                    &final_url,
                    &info.mime_type,
                    source_name,
                    headers,
                    &state,
                    Some((track_id, master_key))
                ).await;
            }

            proxy_stream(&info.url, &info.mime_type, source_name, headers, &state, None).await
        }
        None => (
            StatusCode::BAD_REQUEST,
            Json(RustalinkError::bad_request(
                format!(
                    "Source '{}' could not resolve a stream URL for identifier '{}'",
                    source_name, identifier
                ),
                PATH,
            )),
        )
            .into_response(),
    }
}

async fn stream_youtube(
    video_id: &str,
    itag: Option<i64>,
    with_client: Option<&str>,
    headers: HeaderMap,
    state: &Arc<AppState>,
) -> Response {
    let yt = match &state.source_manager.youtube {
        Some(yt) => yt.clone(),
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(RustalinkError::not_found("YouTube source is not enabled", PATH)),
            )
                .into_response();
        }
    };

    match yt.get_stream_info(video_id, itag, with_client).await {
        Ok(info) => {
            proxy_stream(&info.url, &info.format, "youtube", headers, state, None).await
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(RustalinkError::bad_request(e, PATH)),
        )
            .into_response(),
    }
}

async fn proxy_stream(
    url: &str,
    mime_type: &str,
    source_name: &str,
    incoming_headers: HeaderMap,
    state: &Arc<AppState>,
    deezer_ctx: Option<(String, String)>, // (track_id, master_key)
) -> Response {
    let proxy_config = state.source_manager.get_proxy_config(source_name);
    let http = state.source_manager.http_pool.get(proxy_config);

    let mut builder = http.get(url);

    // Forward Range header if present
    if let Some(range) = incoming_headers.get(header::RANGE) {
        builder = builder.header(header::RANGE, range);
    }

    // Set a reasonable User-Agent if not already present in the client
    builder = builder.header(header::USER_AGENT, "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/119.0.0.0 Safari/537.36");

    let response = match builder.send().await {
        Ok(res) => res,
        Err(e) => {
            tracing::error!("Failed to fetch stream from {}: {}", url, e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(RustalinkError::internal(format!("Failed to fetch upstream: {}", e), PATH)),
            ).into_response();
        }
    };

    let status = response.status();
    let mut res_builder = Response::builder().status(status);

    // Forward essential headers
    if let Some(ct) = response.headers().get(header::CONTENT_TYPE) {
        res_builder = res_builder.header(header::CONTENT_TYPE, ct);
    } else {
        // Fallback to provided mime_type if upstream doesn't provide it
        res_builder = res_builder.header(header::CONTENT_TYPE, mime_type);
    }

    if let Some(cl) = response.headers().get(header::CONTENT_LENGTH) {
        res_builder = res_builder.header(header::CONTENT_LENGTH, cl);
    }

    if let Some(cr) = response.headers().get(header::CONTENT_RANGE) {
        res_builder = res_builder.header(header::CONTENT_RANGE, cr);
    }

    if let Some(at) = response.headers().get(header::ACCEPT_RANGES) {
        res_builder = res_builder.header(header::ACCEPT_RANGES, at);
    }

    let stream = response.bytes_stream().map(|item| item.map_err(|e| e.to_string()));

    let body = if let Some((track_id, master_key)) = deezer_ctx {
        // State for unfold: (underlying_stream, chunk_index, internal_buffer)
        let decryption_state = (stream, 0u64, Vec::with_capacity(CHUNK_SIZE));
        
        let decrypted_stream = futures::stream::unfold(Some(decryption_state), move |state_opt| {
            let track_id_inner = track_id.clone();
            let master_key_inner = master_key.clone();
            async move {
                let (mut stream, mut chunk_index, mut buffer) = state_opt?;
                let crypt = DeezerCrypt::new(&track_id_inner, &master_key_inner);

                while let Some(item) = stream.next().await {
                    match item {
                        Ok(bytes) => {
                            let mut output = Vec::new();
                            let data = bytes.to_vec();
                            let mut remaining = data.as_slice();

                            while !remaining.is_empty() {
                                let to_copy = std::cmp::min(CHUNK_SIZE - buffer.len(), remaining.len());
                                buffer.extend_from_slice(&remaining[..to_copy]);
                                remaining = &remaining[to_copy..];

                                if buffer.len() == CHUNK_SIZE {
                                    crypt.decrypt_chunk(chunk_index, &buffer, &mut output);
                                    chunk_index += 1;
                                    buffer.clear();
                                }
                            }
                            
                            if !output.is_empty() {
                                return Some((Ok::<bytes::Bytes, String>(bytes::Bytes::from(output)), Some((stream, chunk_index, buffer))));
                            }
                        }
                        Err(e) => return Some((Err(e), None)),
                    }
                }

                if !buffer.is_empty() {
                    let final_bytes = bytes::Bytes::from(buffer);
                    return Some((Ok(final_bytes), None));
                }

                None
            }
        });
        
        Body::from_stream(decrypted_stream)
    } else {
        Body::from_stream(stream)
    };

    match res_builder.body(body) {
        Ok(res) => res,
        Err(e) => {
            tracing::error!("Failed to build response: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(RustalinkError::internal("Failed to build response", PATH)),
            ).into_response()
        }
    }
}
