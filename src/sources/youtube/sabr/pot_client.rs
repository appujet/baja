use serde::{Deserialize, Serialize};

/// Response from yt-cipher's /get_pot endpoint.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PoTokenResponse {
    pub visitor_data_token: String,
    pub visitor_data: String,
    pub video_id_token: Option<String>,
    pub expires_at: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PoTokenRequest<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    visitor_data: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    video_id: Option<&'a str>,
}

/// Fetch a PoToken from the yt-cipher service.
///
/// - `base_url`:     URL of the yt-cipher service (e.g. "http://localhost:8001")
/// - `visitor_data`: Optional visitor data used as content binding
/// - `api_token`:    Optional API token sent as `Authorization` header
///                   (matches `API_TOKEN` env var on the server side)
pub async fn fetch_po_token(
    http: &reqwest::Client,
    base_url: &str,
    visitor_data: Option<&str>,
    video_id: Option<&str>,
    api_token: Option<&str>,
) -> Option<PoTokenResponse> {
    let url = format!("{}/get_pot", base_url.trim_end_matches('/'));
    let req_body = PoTokenRequest {
        visitor_data,
        video_id,
    };

    let mut req = http.post(&url).json(&req_body);

    if let Some(token) = api_token {
        req = req.header("authorization", token);
    }

    let res = req
        .send()
        .await
        .map_err(|e| {
            tracing::warn!("SABR PoToken: request to {} failed: {}", url, e);
        })
        .ok()?;

    if !res.status().is_success() {
        tracing::warn!("SABR PoToken: {} returned HTTP {}", url, res.status());
        return None;
    }

    res.json::<PoTokenResponse>()
        .await
        .map_err(|e| {
            tracing::warn!("SABR PoToken: failed to parse response: {}", e);
        })
        .ok()
}
