//! Player API helper for SABR.
//!
//! Calls the YouTube Innertube player endpoint and extracts a SabrConfig
//! from the response. This lives inside the sabr/ module so the SABR
//! implementation stays fully self-contained.

use serde_json::{Value, json};

use super::{config::SabrConfig, pot_client};

const INNERTUBE_API: &str = "https://youtubei.googleapis.com";

/// Call the Innertube player API with the WEB client and parse a `SabrConfig`
/// if the response contains `serverAbrStreamingUrl`.
///
/// Every failure path is explicitly logged at DEBUG level so problems are visible.
pub async fn fetch_sabr_config(
    http: &reqwest::Client,
    video_id: &str,
    visitor_data: Option<&str>,
    po_token: Option<String>,
    signature_timestamp: Option<u32>,
    client_name_id: i32,
    client_name_str: &str,
    client_version: &str,
    user_agent: &str,
    yt_cipher_url: Option<&str>,
    api_token: Option<&str>,
) -> Option<SabrConfig> {
    // ── PoToken ─────────────────────────────────────────────────────────────
    // visitor_data may come from caller OR from PoToken service.
    // Uses the PoToken's visitorData as X-Goog-Visitor-Id exactly.
    let (po_token, effective_visitor_data): (Option<String>, Option<String>) = match po_token {
        Some(pt) => {
            tracing::debug!("SABR player[{}]: using provided PoToken", video_id);
            (Some(pt), visitor_data.map(str::to_string))
        }
        None => {
            if let Some(cipher_url) = yt_cipher_url {
                tracing::debug!(
                    "SABR player[{}]: fetching PoToken from {}",
                    video_id,
                    cipher_url
                );
                // For SABR streaming, the content binding must be the video_id
                let pot_resp = pot_client::fetch_po_token(
                    http,
                    cipher_url,
                    visitor_data,
                    Some(video_id),
                    api_token,
                )
                .await;
                match pot_resp {
                    Some(ref r) => {
                        let pt = r
                            .video_id_token
                            .clone()
                            .unwrap_or_else(|| r.visitor_data_token.clone());
                        tracing::debug!(
                            "SABR player[{}]: got PoToken (len={}) visitorData={}",
                            video_id,
                            pt.len(),
                            r.visitor_data.len()
                        );
                        // Use PoToken's visitorData
                        let vd = if r.visitor_data.is_empty() {
                            visitor_data.map(str::to_string)
                        } else {
                            Some(r.visitor_data.clone())
                        };
                        (Some(pt), vd)
                    }
                    None => {
                        tracing::debug!(
                            "SABR player[{}]: PoToken fetch failed — proceeding without",
                            video_id
                        );
                        (None, visitor_data.map(str::to_string))
                    }
                }
            } else {
                tracing::debug!(
                    "SABR player[{}]: no yt_cipher_url configured — no PoToken",
                    video_id
                );
                (None, visitor_data.map(str::to_string))
            }
        }
    };

    // ── Build Innertube player request ───────────────────────────────────────
    let mut client_obj = json!({
        "clientName": client_name_str,
        "clientVersion": client_version,
        "userAgent": user_agent,
        "platform": "DESKTOP",
        "hl": "en",
        "gl": "US"
    });

    // Insert visitorData into context.client
    if let Some(vd) = &effective_visitor_data {
        if let Some(obj) = client_obj.as_object_mut() {
            obj.insert("visitorData".to_string(), vd.clone().into());
        }
    }

    let mut body = json!({
        "context": {
            "client": client_obj,
            "user": { "lockedSafetyMode": false },
            "request": { "useSsl": true }
        },
        "videoId": video_id,
        "contentCheckOk": true,
        "racyCheckOk": true
    });

    if let Some(pt) = &po_token {
        if let Some(obj) = body.as_object_mut() {
            obj.insert(
                "serviceIntegrityDimensions".to_string(),
                json!({ "poToken": pt }),
            );
        }
    }

    if let Some(sts) = signature_timestamp {
        if let Some(obj) = body.as_object_mut() {
            obj.insert(
                "playbackContext".to_string(),
                json!({
                    "contentPlaybackContext": {
                        "signatureTimestamp": sts
                    }
                }),
            );
        }
    }

    let url = format!("{}/youtubei/v1/player?prettyPrint=false", INNERTUBE_API);
    tracing::debug!(
        "SABR player[{}]: POST {} (sts={:?} hasPoToken={} hasVisitorData={})",
        video_id,
        url,
        signature_timestamp,
        po_token.is_some(),
        effective_visitor_data.is_some()
    );

    // headers (Web.js lines 294-300):
    //   'User-Agent', 'X-Goog-Visitor-Id', 'X-Youtube-Client-Name',
    //   'X-Youtube-Client-Version', 'Origin'
    let mut req = http
        .post(&url)
        .header("X-YouTube-Client-Name", client_name_id.to_string())
        .header("X-YouTube-Client-Version", client_version)
        .header("Origin", "https://www.youtube.com")
        .header(
            "Referer",
            format!("https://www.youtube.com/watch?v={}", video_id),
        );

    // X-Goog-Visitor-Id: always send if we have visitorData (always does)
    if let Some(vd) = &effective_visitor_data {
        req = req.header("X-Goog-Visitor-Id", vd.as_str());
    }

    let res = match req.json(&body).send().await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("SABR player[{}]: network error: {}", video_id, e);
            return None;
        }
    };

    let status = res.status();
    tracing::debug!(
        "SABR player[{}]: Innertube response HTTP {}",
        video_id,
        status
    );

    if !status.is_success() {
        let text = res.text().await.unwrap_or_default();
        tracing::warn!(
            "SABR player[{}]: HTTP {} from Innertube: {}",
            video_id,
            status,
            &text[..text.len().min(200)]
        );
        return None;
    }

    let response: Value = match res.json().await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                "SABR player[{}]: failed to parse Innertube JSON: {}",
                video_id,
                e
            );
            return None;
        }
    };

    let playability = response
        .get("playabilityStatus")
        .and_then(|p| p.get("status"))
        .and_then(|s| s.as_str())
        .unwrap_or("UNKNOWN");

    tracing::debug!(
        "SABR player[{}]: playabilityStatus={}",
        video_id,
        playability
    );

    if playability != "OK" {
        let reason = response
            .get("playabilityStatus")
            .and_then(|p| p.get("reason"))
            .and_then(|r| r.as_str())
            .unwrap_or("(no reason)");
        tracing::debug!(
            "SABR player[{}]: not playable (status={} reason={})",
            video_id,
            playability,
            reason
        );
        return None;
    }

    let has_sabr = response
        .get("streamingData")
        .and_then(|sd| sd.get("serverAbrStreamingUrl"))
        .and_then(|u| u.as_str());

    match has_sabr {
        Some(sabr_url) => {
            tracing::debug!(
                "SABR player[{}]: serverAbrStreamingUrl found: {}",
                video_id,
                &sabr_url[..sabr_url.len().min(80)]
            );
        }
        None => {
            tracing::debug!(
                "SABR player[{}]: no serverAbrStreamingUrl in response — not a SABR video",
                video_id
            );
            return None;
        }
    }

    match SabrConfig::from_player_response(
        &response,
        effective_visitor_data,
        po_token,
        client_name_id,
        client_version.to_string(),
        user_agent.to_string(),
    ) {
        Some(cfg) => {
            tracing::info!(
                "SABR player[{}]: SabrConfig ready — {} formats available",
                video_id,
                cfg.formats.len()
            );
            Some(cfg)
        }
        None => {
            tracing::warn!(
                "SABR player[{}]: SabrConfig::from_player_response returned None (no suitable audio format?)",
                video_id
            );
            None
        }
    }
}
