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

use std::sync::Arc;

use serde_json::{Value, json};
use tokio::sync::RwLock;
use tracing::{debug, warn};

use super::{
    region::DomainConfigCache,
    validators::{is_invalid_album, is_invalid_playlist},
};

const CONFIG_URL: &str = "https://music.amazon.com/config.json";
const API_BASE: &str = "https://eu.mesk.skill.music.a2z.com/api";

pub const EP_TRACK_INFO: &str = "cosmicTrack/displayCatalogTrack";
pub const EP_ALBUM_INFO: &str = "showCatalogAlbum";
pub const EP_ARTIST_INFO: &str = "explore/v1/showCatalogArtist";
pub const EP_PLAYLIST_INFO: &str = "showCatalogPlaylist";
pub const EP_COMMUNITY_PLAYLIST_INFO: &str = "showLibraryPlaylist";
pub const EP_TRACKS_SEARCH: &str = "searchCatalogTracks";

const USER_AGENT: &str = "Mozilla/5.0 (Linux; Android 10; K) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Mobile Safari/537.36";

pub struct AmazonMusicClient {
    http: Arc<reqwest::Client>,
    cached_config: RwLock<Option<Value>>,
    domain_configs: Arc<DomainConfigCache>,
}

impl AmazonMusicClient {
    pub fn new(http: Arc<reqwest::Client>) -> Self {
        Self {
            http,
            cached_config: RwLock::new(None),
            domain_configs: Arc::new(DomainConfigCache::new()),
        }
    }

    pub async fn site_config(&self) -> Option<Value> {
        {
            let guard = self.cached_config.read().await;
            if guard.is_some() {
                return guard.clone();
            }
        }

        let resp = match self
            .http
            .get(CONFIG_URL)
            .header("accept", "*/*")
            .header("accept-language", "en-US,en;q=0.9")
            .header("origin", "https://music.amazon.com")
            .header("referer", "https://music.amazon.com/")
            .header("user-agent", USER_AGENT)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                warn!("Amazon Music: config fetch failed: {e}");
                return None;
            }
        };

        if !resp.status().is_success() {
            warn!("Amazon Music: config fetch HTTP {}", resp.status());
            return None;
        }

        let config: Value = match resp.json().await {
            Ok(v) => v,
            Err(e) => {
                debug!("Amazon Music: config JSON parse error: {e}");
                return None;
            }
        };

        *self.cached_config.write().await = Some(config.clone());
        Some(config)
    }

    pub fn build_amzn_headers(config: &Value, page_url: &str) -> Value {
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

        let req_id = gen_request_id();

        json!({
            "x-amzn-authentication": serde_json::to_string(&json!({
                "interface": "ClientAuthenticationInterface.v1_0.ClientTokenElement",
                "accessToken": access_token
            })).unwrap_or_default(),
            "x-amzn-device-model": "WEBPLAYER",
            "x-amzn-device-width": "1920",
            "x-amzn-device-family": "WebPlayer",
            "x-amzn-device-id": device_id,
            "x-amzn-user-agent": USER_AGENT,
            "x-amzn-session-id": session_id,
            "x-amzn-device-height": "1080",
            "x-amzn-request-id": req_id,
            "x-amzn-device-language": "en_US",
            "x-amzn-currency-of-preference": "USD",
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
            "x-amzn-music-domain": "music.amazon.com",
            "x-amzn-referer": "music.amazon.com",
            "x-amzn-affiliate-tags": "",
            "x-amzn-ref-marker": "",
            "x-amzn-page-url": page_url,
            "x-amzn-weblab-id-overrides": "",
            "x-amzn-video-player-token": "",
            "x-amzn-feature-flags": "hd-supported,uhd-supported",
            "x-amzn-has-profile-id": "",
            "x-amzn-age-band": ""
        })
    }

    pub async fn post_endpoint(
        &self,
        path: &str,
        mut body: Value,
        page_url: &str,
    ) -> Option<Value> {
        let config = self.site_config().await?;
        let amzn_headers = Self::build_amzn_headers(&config, page_url);

        body["headers"] = Value::String(serde_json::to_string(&amzn_headers).unwrap_or_default());

        let url = format!("{API_BASE}/{path}");

        let resp = match self
            .http
            .post(&url)
            .header("authority", "eu.mesk.skill.music.a2z.com")
            .header("accept", "*/*")
            .header("accept-language", "en-US,en;q=0.9")
            .header("content-type", "text/plain;charset=UTF-8")
            .header("origin", "https://music.amazon.com")
            .header("referer", "https://music.amazon.com/")
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
            .body(serde_json::to_string(&body).unwrap_or_default())
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                debug!("Amazon Music: POST {path} failed: {e}");
                return None;
            }
        };

        if !resp.status().is_success() {
            warn!("Amazon Music: POST {path} HTTP {}", resp.status());
            *self.cached_config.write().await = None;
            return None;
        }

        match resp.json::<Value>().await {
            Ok(v) => Some(v),
            Err(e) => {
                debug!("Amazon Music: POST {path} JSON parse error: {e}");
                None
            }
        }
    }

    fn entity_body(id: &str) -> Value {
        json!({
            "id": id,
            "userHash": serde_json::to_string(&json!({"level": "LIBRARY_MEMBER"})).unwrap_or_default()
        })
    }

    pub async fn fetch_track(&self, id: &str) -> Option<Value> {
        let page = format!(
            "https://music.amazon.com/tracks/{}",
            urlencoding::encode(id)
        );
        self.post_endpoint(EP_TRACK_INFO, Self::entity_body(id), &page)
            .await
    }

    pub async fn fetch_album(&self, id: &str) -> Option<Value> {
        let page = format!(
            "https://music.amazon.com/albums/{}",
            urlencoding::encode(id)
        );
        self.post_endpoint(EP_ALBUM_INFO, Self::entity_body(id), &page)
            .await
    }

    pub async fn fetch_artist(&self, id: &str) -> Option<Value> {
        let page = format!(
            "https://music.amazon.com/artists/{}",
            urlencoding::encode(id)
        );
        self.post_endpoint(EP_ARTIST_INFO, Self::entity_body(id), &page)
            .await
    }

    pub async fn fetch_playlist(&self, id: &str) -> Option<Value> {
        let page = format!(
            "https://music.amazon.com/playlists/{}",
            urlencoding::encode(id)
        );
        self.post_endpoint(EP_PLAYLIST_INFO, Self::entity_body(id), &page)
            .await
    }

    pub async fn fetch_community_playlist(&self, id: &str) -> Option<Value> {
        let page = format!(
            "https://music.amazon.com/user-playlists/{}",
            urlencoding::encode(id)
        );
        self.post_endpoint(EP_COMMUNITY_PLAYLIST_INFO, Self::entity_body(id), &page)
            .await
    }

    pub async fn search_tracks(&self, query: &str) -> Option<Value> {
        let page = format!(
            "https://music.amazon.com/search/{}/songs",
            urlencoding::encode(query)
        );
        let body = json!({
            "keyword": query,
            "userHash": serde_json::to_string(&json!({"level": "LIBRARY_MEMBER"})).unwrap_or_default()
        });
        self.post_endpoint(EP_TRACKS_SEARCH, body, &page).await
    }

    pub async fn fetch_album_multi_region(
        &self,
        id: &str,
        domain_hint: Option<&str>,
    ) -> Option<Value> {
        super::region::fetch_multi_region(
            &self.http,
            id,
            EP_ALBUM_INFO,
            "albums",
            "Album",
            domain_hint,
            is_invalid_album,
            &self.domain_configs,
        )
        .await
    }

    pub async fn fetch_playlist_multi_region(
        &self,
        id: &str,
        domain_hint: Option<&str>,
    ) -> Option<Value> {
        super::region::fetch_multi_region(
            &self.http,
            id,
            EP_PLAYLIST_INFO,
            "playlists",
            "Playlist",
            domain_hint,
            is_invalid_playlist,
            &self.domain_configs,
        )
        .await
    }
}

pub fn gen_request_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(42);
    let mut x = seed ^ (seed << 13);
    x ^= x >> 17;
    x ^= x << 5;
    (0..13)
        .map(|i| {
            x ^= x.wrapping_add(i).wrapping_mul(1664525);
            b"0123456789abcdefghijklmnopqrstuvwxyz"[(x as usize) % 36] as char
        })
        .collect()
}

pub fn duration_str_to_ms(s: &str) -> u64 {
    let s = s.trim();
    if s.is_empty() {
        return 0;
    }

    let parts: Vec<u64> = s.split(':').filter_map(|p| p.trim().parse().ok()).collect();

    let secs = match parts.as_slice() {
        [h, m, s] => h * 3600 + m * 60 + s,
        [first, second] => {
            if *first >= 60 {
                let h = first / 60;
                let m = first % 60;
                h * 3600 + m * 60 + second
            } else {
                first * 60 + second
            }
        }
        [n] => *n,
        _ => 0,
    };

    secs * 1000
}

pub fn clean_image_url(url: &str) -> String {
    if url.is_empty() {
        return url.to_owned();
    }

    if url.contains("_CLa%7C") {
        return url
            .replace("._AA", ".")
            .replace("_US354", "_US1000")
            .replace("CLa%7C354,354", "CLa%7C1000,1000")
            .replace("0,0,354,354", "0,0,1000,1000")
            .replace("0,0,177,177", "0,0,500,500")
            .replace("177,0,177,177", "500,0,500,500")
            .replace("0,177,177,177", "0,500,500,500")
            .replace("177,177,177,177", "500,500,500,500");
    }

    if let Some(i_pos) = url.find("/I/") {
        let after = &url[i_pos + 3..];
        if let Some(dot_pos) = after.rfind('.') {
            let ext = &after[dot_pos..];
            let id_end = after.find(&['.', '_', '?'][..]).unwrap_or(after.len());
            let id_part = &after[..id_end];
            return format!("{}/I/{}{}", &url[..i_pos], id_part, ext);
        }
    }

    url.to_owned()
}

pub fn clean_song_title(title: &str) -> String {
    if let Some(stripped) = title.trim().strip_prefix(|c: char| c.is_ascii_digit()) {
        let rest = stripped.trim_start_matches(|c: char| c.is_ascii_digit());
        if let Some(clean) = rest.strip_prefix(". ") {
            return clean.to_owned();
        }
    }
    title.to_owned()
}

pub fn normalize_artist(raw: &str) -> String {
    let parts: Vec<&str> = raw
        .split(['&', ','])
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .take(3)
        .collect();

    if parts.is_empty() {
        return raw.trim().to_owned();
    }
    parts.join(", ")
}
