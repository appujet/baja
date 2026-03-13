// Copyright (c) 2026 appujet, notdeltaxd and contributors
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::{sync::LazyLock, time::Duration};

use futures::{StreamExt, stream::FuturesUnordered};
use regex::Regex;
use serde_json::{Value, json};
use tracing::{debug, warn};

use super::api::gen_request_id;

const USER_AGENT: &str = "Mozilla/5.0 (Linux; Android 10; K) AppleWebKit/537.36 \
    (KHTML, like Gecko) Chrome/125.0.0.0 Mobile Safari/537.36";

pub struct RegionConfig {
    pub skill_endpoint: &'static str,
    pub language: &'static str,
    pub currency: &'static str,
    pub device_family: &'static str,
    pub feature_flags: &'static str,
}

const EP_NA: &str = "https://na.web.skill.music.a2z.com";
const EP_EU: &str = "https://eu.web.skill.music.a2z.com";
const EP_FE: &str = "https://fe.web.skill.music.a2z.com";

static REGION_CONFIGS: &[(&str, RegionConfig)] = &[
    (
        "music.amazon.com",
        RegionConfig {
            skill_endpoint: EP_NA,
            language: "en_US",
            currency: "USD",
            device_family: "WebPlayer",
            feature_flags: "",
        },
    ),
    (
        "music.amazon.com.mx",
        RegionConfig {
            skill_endpoint: EP_NA,
            language: "es_MX",
            currency: "MXN",
            device_family: "WebPlayer",
            feature_flags: "",
        },
    ),
    (
        "music.amazon.com.br",
        RegionConfig {
            skill_endpoint: EP_NA,
            language: "pt_BR",
            currency: "BRL",
            device_family: "WebPlayer",
            feature_flags: "",
        },
    ),
    (
        "music.amazon.ca",
        RegionConfig {
            skill_endpoint: EP_NA,
            language: "en_CA",
            currency: "CAD",
            device_family: "WebPlayer",
            feature_flags: "",
        },
    ),
    (
        "music.amazon.co.uk",
        RegionConfig {
            skill_endpoint: EP_EU,
            language: "en_GB",
            currency: "GBP",
            device_family: "WebPlayer",
            feature_flags: "hd-supported,uhd-supported",
        },
    ),
    (
        "music.amazon.de",
        RegionConfig {
            skill_endpoint: EP_EU,
            language: "de_DE",
            currency: "EUR",
            device_family: "WebPlayer",
            feature_flags: "hd-supported,uhd-supported",
        },
    ),
    (
        "music.amazon.fr",
        RegionConfig {
            skill_endpoint: EP_EU,
            language: "fr_FR",
            currency: "EUR",
            device_family: "WebPlayer",
            feature_flags: "hd-supported,uhd-supported",
        },
    ),
    (
        "music.amazon.it",
        RegionConfig {
            skill_endpoint: EP_EU,
            language: "it_IT",
            currency: "EUR",
            device_family: "WebPlayer",
            feature_flags: "hd-supported,uhd-supported",
        },
    ),
    (
        "music.amazon.es",
        RegionConfig {
            skill_endpoint: EP_EU,
            language: "es_ES",
            currency: "EUR",
            device_family: "WebPlayer",
            feature_flags: "hd-supported,uhd-supported",
        },
    ),
    (
        "music.amazon.in",
        RegionConfig {
            skill_endpoint: EP_EU,
            language: "en_IN",
            currency: "INR",
            device_family: "WebPlayer",
            feature_flags: "hd-supported,uhd-supported",
        },
    ),
    (
        "music.amazon.sa",
        RegionConfig {
            skill_endpoint: EP_EU,
            language: "ar_SA",
            currency: "SAR",
            device_family: "WebPlayer",
            feature_flags: "",
        },
    ),
    (
        "music.amazon.ae",
        RegionConfig {
            skill_endpoint: EP_EU,
            language: "ar_AE",
            currency: "AED",
            device_family: "WebPlayer",
            feature_flags: "",
        },
    ),
    (
        "music.amazon.co.jp",
        RegionConfig {
            skill_endpoint: EP_FE,
            language: "ja_JP",
            currency: "JPY",
            device_family: "WebPlayer",
            feature_flags: "hd-supported,uhd-supported",
        },
    ),
    (
        "music.amazon.com.au",
        RegionConfig {
            skill_endpoint: EP_FE,
            language: "en_AU",
            currency: "AUD",
            device_family: "WebPlayer",
            feature_flags: "hd-supported,uhd-supported",
        },
    ),
];

static REGION_FALLBACKS: &[(&str, &str)] = &[
    ("NA", "music.amazon.com"),
    ("EU", "music.amazon.co.uk"),
    ("FE", "music.amazon.co.jp"),
];

fn get_region_config(domain: &str) -> Option<&'static RegionConfig> {
    REGION_CONFIGS
        .iter()
        .find(|(d, _)| *d == domain)
        .map(|(_, cfg)| cfg)
}

static DOMAIN_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^https?://(?:www\.)?(music\.amazon\.[a-z.]+)").unwrap());

pub fn extract_domain(url: &str) -> Option<String> {
    let caps = DOMAIN_RE.captures(url)?;
    let domain = caps.get(1)?.as_str().to_lowercase();
    if get_region_config(&domain).is_some() {
        Some(domain)
    } else {
        None
    }
}

fn build_region_headers(
    config: &Value,
    region: &RegionConfig,
    domain: &str,
    page_url: &str,
) -> Value {
    let access_token = config["accessToken"].as_str().unwrap_or("");
    let device_id = config["deviceId"].as_str().unwrap_or("");
    let session_id = config["sessionId"].as_str().unwrap_or("");
    let version = config["version"].as_str().unwrap_or("");
    let csrf_token = config["csrf"]["token"].as_str().unwrap_or("");
    let csrf_ts = config["csrf"]["ts"]
        .as_str()
        .map(|s| s.to_string())
        .or_else(|| config["csrf"]["ts"].as_u64().map(|n| n.to_string()))
        .unwrap_or_default();
    let csrf_rnd = config["csrf"]["rnd"]
        .as_str()
        .map(|s| s.to_string())
        .or_else(|| config["csrf"]["rnd"].as_u64().map(|n| n.to_string()))
        .unwrap_or_default();

    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis().to_string())
        .unwrap_or_default();

    json!({
        "x-amzn-authentication": serde_json::to_string(&json!({
            "interface": "ClientAuthenticationInterface.v1_0.ClientTokenElement",
            "accessToken": access_token
        })).unwrap_or_default(),
        "x-amzn-device-model": "WEBPLAYER",
        "x-amzn-device-width": "1920",
        "x-amzn-device-family": region.device_family,
        "x-amzn-device-id": device_id,
        "x-amzn-user-agent": USER_AGENT,
        "x-amzn-session-id": session_id,
        "x-amzn-device-height": "1080",
        "x-amzn-request-id": gen_request_id(),
        "x-amzn-device-language": region.language,
        "x-amzn-currency-of-preference": region.currency,
        "x-amzn-os-version": "1.0",
        "x-amzn-application-version": version,
        "x-amzn-device-time-zone": "Asia/Calcutta",
        "x-amzn-timestamp": ts,
        "x-amzn-csrf": serde_json::to_string(&json!({
            "interface": "CSRFInterface.v1_0.CSRFHeaderElement",
            "token": csrf_token,
            "timestamp": csrf_ts,
            "rndNonce": csrf_rnd
        })).unwrap_or_default(),
        "x-amzn-music-domain": domain,
        "x-amzn-referer": domain,
        "x-amzn-affiliate-tags": "",
        "x-amzn-ref-marker": "",
        "x-amzn-page-url": page_url,
        "x-amzn-weblab-id-overrides": "",
        "x-amzn-video-player-token": "",
        "x-amzn-feature-flags": region.feature_flags,
        "x-amzn-has-profile-id": "",
        "x-amzn-age-band": ""
    })
}

async fn fetch_from_endpoint(
    http: &reqwest::Client,
    id: &str,
    api_path: &str,
    url_path_segment: &str,
    region: &RegionConfig,
    domain: &str,
    base_config: &Value,
) -> Option<Value> {
    let page_url = format!(
        "https://{domain}/{url_path_segment}/{}",
        urlencoding::encode(id)
    );
    let inner_headers = build_region_headers(base_config, region, domain, &page_url);

    let body = json!({
        "id": id,
        "userHash": serde_json::to_string(&json!({"level": "LIBRARY_MEMBER"})).unwrap_or_default(),
        "headers": serde_json::to_string(&inner_headers).unwrap_or_default()
    });

    let endpoint_url = format!("{}/api/{api_path}", region.skill_endpoint);

    let authority = region
        .skill_endpoint
        .trim_start_matches("https://")
        .trim_start_matches("http://");

    let resp = http
        .post(&endpoint_url)
        .header("authority", authority)
        .header("accept", "*/*")
        .header("accept-language", "en-US,en;q=0.9")
        .header("content-type", "text/plain;charset=UTF-8")
        .header("origin", format!("https://{domain}"))
        .header("referer", format!("https://{domain}/"))
        .header(
            "sec-ch-ua",
            "\"Chromium\";v=\"125\", \"Not.A/Brand\";v=\"24\"",
        )
        .header("sec-ch-ua-mobile", "?1")
        .header("sec-ch-ua-platform", "\"Android\"")
        .header("sec-fetch-dest", "empty")
        .header("sec-fetch-mode", "cors")
        .header("sec-fetch-site", "cross-site")
        .header("user-agent", USER_AGENT)
        .timeout(Duration::from_secs(8))
        .body(serde_json::to_string(&body).unwrap_or_default())
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }

    resp.json::<Value>().await.ok()
}

#[allow(clippy::too_many_arguments)]
pub async fn fetch_multi_region(
    http: &reqwest::Client,
    id: &str,
    api_path: &str,
    url_path_segment: &str,
    entity_name: &str,
    domain_hint: Option<&str>,
    is_error: fn(&Value) -> bool,
    base_config: &Value,
) -> Option<Value> {
    let mut tried_endpoint: Option<&str> = None;

    if let Some(hint) = domain_hint
        && let Some(region) = get_region_config(hint)
    {
        debug!("Amazon Music: trying {entity_name} on hinted domain '{hint}'");
        tried_endpoint = Some(region.skill_endpoint);
        if let Some(data) = fetch_from_endpoint(
            http,
            id,
            api_path,
            url_path_segment,
            region,
            hint,
            base_config,
        )
        .await
            && !is_error(&data)
        {
            debug!("Amazon Music: {entity_name} resolved via hinted domain '{hint}'");
            return Some(data);
        }
        debug!(
            "Amazon Music: hinted domain '{hint}' failed for {entity_name}, trying other regions"
        );
    }

    let mut futs = FuturesUnordered::new();

    for &(label, domain) in REGION_FALLBACKS {
        let region = match get_region_config(domain) {
            Some(r) => r,
            None => continue,
        };
        if tried_endpoint == Some(region.skill_endpoint) {
            continue;
        }
        // Clone base config reference for the async move block
        let base_config = base_config.clone();
        futs.push(async move {
            let result = fetch_from_endpoint(
                http,
                id,
                api_path,
                url_path_segment,
                region,
                domain,
                &base_config,
            )
            .await;
            result.map(|data| (data, label))
        });
    }

    while let Some(result) = futs.next().await {
        if let Some((data, label)) = result
            && !is_error(&data)
        {
            debug!("Amazon Music: {entity_name} resolved via {label} region");
            return Some(data);
        }
    }

    warn!("Amazon Music: {entity_name} not found on any regional endpoint (NA, EU, FE)");
    None
}
