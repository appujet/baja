use std::sync::Arc;
use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose};
use regex::Regex;
use serde_json::{Value, json};

use crate::{
    api::tracks::{LoadResult, PlaylistData, PlaylistInfo, Track, TrackInfo},
    sources::{SourcePlugin, plugin::BoxedTrack},
};

pub mod track;
pub mod reader;

const DECRYPTION_KEY: &[u8] = b"IFYOUWANTTHEARTISTSTOGETPAIDDONOTDOWNLOADFROMMIXCLOUD";
const GRAPHQL_URL: &str = "https://app.mixcloud.com/graphql";

pub struct MixcloudSource {
    client: reqwest::Client,
    track_url_re: Regex,
    playlist_url_re: Regex,
    user_url_re: Regex,
    search_prefixes: Vec<String>,
    search_limit: usize,
}

impl MixcloudSource {
    pub fn new(config: Option<crate::configs::MixcloudConfig>) -> Result<Self, String> {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::USER_AGENT,
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36".parse().unwrap()
        );

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .build()
            .map_err(|e| e.to_string())?;

        Ok(Self {
            client,
            track_url_re: Regex::new(
                r"(?i)^https?://(?:(?:www|beta|m)\.)?mixcloud\.com/(?P<user>[^/]+)/(?P<slug>[^/]+)/?$",
            )
            .unwrap(),
            playlist_url_re: Regex::new(
                r"(?i)^https?://(?:(?:www|beta|m)\.)?mixcloud\.com/(?P<user>[^/]+)/playlists/(?P<playlist>[^/]+)/?$",
            )
            .unwrap(),
            user_url_re: Regex::new(
                r"(?i)^https?://(?:(?:www|beta|m)\.)?mixcloud\.com/(?P<id>[^/]+)(?:/(?P<type>uploads|favorites|listens|stream))?/?$",
            )
            .unwrap(),
            search_prefixes: vec!["mcsearch:".to_string()],
            search_limit: config.map(|c| c.search_limit).unwrap_or(10),
        })
    }
}

pub fn decrypt(ciphertext_b64: &str) -> String {
        let ciphertext: Vec<u8> = match general_purpose::STANDARD.decode(ciphertext_b64) {
            Ok(b) => b,
            Err(_) => return String::new(),
        };

        let mut decrypted = Vec::with_capacity(ciphertext.len());
        for (i, &byte) in ciphertext.iter().enumerate() {
            decrypted.push(byte ^ DECRYPTION_KEY[i % DECRYPTION_KEY.len()]);
        }

        String::from_utf8(decrypted).unwrap_or_default()
}

impl MixcloudSource {
    async fn graphql_request(&self, query: &str) -> Option<Value> {
        let url = format!("{}?query={}", GRAPHQL_URL, urlencoding::encode(query));
        let resp = self.client.get(url).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }
        resp.json::<Value>().await.ok()
    }

    fn parse_track_data(&self, data: &Value) -> Option<Track> {
        let url_raw = data["url"].as_str()?;
        let path_parts: Vec<&str> = url_raw
            .split("mixcloud.com/")
            .nth(1)?
            .split('/')
            .filter(|s| !s.is_empty())
            .collect();

        if path_parts.len() < 2 {
            return None;
        }

        let id = format!("{}_{}", path_parts[0], path_parts[1]);
        let title = data["name"].as_str()?.to_string();
        let author = data["owner"]["displayName"]
            .as_str()
            .or_else(|| Some(path_parts[0]))?
            .to_string();
        let duration_ms = (data["audioLength"].as_u64().unwrap_or(0)) * 1000;
        let artwork_url = data["picture"]["url"].as_str().map(|s| s.to_string());

        let track = Track::new(TrackInfo {
            identifier: id,
            is_seekable: true,
            author,
            length: duration_ms,
            is_stream: false,
            position: 0,
            title,
            uri: Some(url_raw.to_string()),
            artwork_url,
            isrc: None,
            source_name: "mixcloud".to_string(),
        });

        Some(track)
    }

    async fn resolve_track(&self, username: &str, slug: &str) -> LoadResult {
        let query = format!(
            "{{
        cloudcastLookup(lookup: {{username: \"{}\", slug: \"{}\"}}) {{
          audioLength
          name
          url
          owner {{ displayName username }}
          picture(width: 1024, height: 1024) {{ url }}
          streamInfo {{ hlsUrl url }}
          restrictedReason
        }}
      }}",
            username, slug
        );

        match self.graphql_request(&query).await {
            Some(body) => {
                if let Some(data) = body["data"]["cloudcastLookup"].as_object() {
                    if let Some(reason) = data.get("restrictedReason").and_then(|v| v.as_str()) {
                        return LoadResult::Error(crate::api::tracks::LoadError {
                            message: format!("Track restricted: {}", reason),
                            severity: crate::common::Severity::Common,
                            cause: reason.to_string(),
                        });
                    }

                    if let Some(track) = self.parse_track_data(&Value::Object(data.clone())) {
                        return LoadResult::Track(track);
                    }
                }
                LoadResult::Empty {}
            }
            None => LoadResult::Empty {},
        }
    }

    async fn resolve_playlist(&self, user: &str, slug: &str) -> LoadResult {
        let query_template = |cursor: Option<&str>| {
            format!(
                "{{
        playlistLookup(lookup: {{username: \"{}\", slug: \"{}\"}}) {{
          name
          items(first: 100{}) {{
            edges {{
              node {{
                cloudcast {{
                  audioLength
                  name
                  url
                  owner {{ displayName username }}
                  picture(width: 1024, height: 1024) {{ url }}
                  streamInfo {{ hlsUrl url }}
                }}
              }}
            }}
            pageInfo {{ endCursor hasNextPage }}
          }}
        }}
      }}",
                user,
                slug,
                cursor.map(|c| format!(", after: \"{}\"", c)).unwrap_or_default()
            )
        };

        let mut tracks = Vec::new();
        let mut cursor: Option<String> = None;
        let mut playlist_name = "Mixcloud Playlist".to_string();

        loop {
            let query = query_template(cursor.as_deref());
            let body = match self.graphql_request(&query).await {
                Some(b) => b,
                None => break,
            };

            let lookup = &body["data"]["playlistLookup"];
            if lookup.is_null() {
                break;
            }

            if let Some(name) = lookup["name"].as_str() {
                playlist_name = name.to_string();
            }

            if let Some(edges) = lookup["items"]["edges"].as_array() {
                for edge in edges {
                    if let Some(track) = self.parse_track_data(&edge["node"]["cloudcast"]) {
                        tracks.push(track);
                    }
                }
            }

            if lookup["items"]["pageInfo"]["hasNextPage"].as_bool() == Some(true) {
                cursor = lookup["items"]["pageInfo"]["endCursor"]
                    .as_str()
                    .map(|s| s.to_string());
                if cursor.is_none() || tracks.len() >= 1000 {
                    break;
                }
            } else {
                break;
            }
        }

        if tracks.is_empty() {
            return LoadResult::Empty {};
        }

        LoadResult::Playlist(PlaylistData {
            info: PlaylistInfo {
                name: playlist_name,
                selected_track: -1,
            },
            plugin_info: json!({}),
            tracks,
        })
    }

    async fn resolve_user(&self, username: &str, list_type: &str) -> LoadResult {
        let query_type = if list_type == "stream" { "stream" } else { list_type };
        let node_query = if list_type == "stream" {
            "... on Cloudcast { audioLength name url owner { displayName username } picture(width: 1024, height: 1024) { url } streamInfo { hlsUrl url } }"
        } else {
            "audioLength name url owner { displayName username } picture(width: 1024, height: 1024) { url } streamInfo { hlsUrl url }"
        };

        let query_template = |cursor: Option<&str>| {
            format!(
                "{{
        userLookup(lookup: {{username: \"{}\"}}) {{
          displayName
          {}(first: 100{}) {{
            edges {{
              node {{
                {}
              }}
            }}
            pageInfo {{ endCursor hasNextPage }}
          }}
        }}
      }}",
                username,
                query_type,
                cursor.map(|c| format!(", after: \"{}\"", c)).unwrap_or_default(),
                node_query
            )
        };

        let mut tracks = Vec::new();
        let mut cursor: Option<String> = None;
        let mut display_name = username.to_string();

        loop {
            let query = query_template(cursor.as_deref());
            let body = match self.graphql_request(&query).await {
                Some(b) => b,
                None => break,
            };

            let lookup = &body["data"]["userLookup"];
            if lookup.is_null() {
                break;
            }

            display_name = lookup["displayName"]
                .as_str()
                .unwrap_or(username)
                .to_string();

            if let Some(edges) = lookup[query_type]["edges"].as_array() {
                for edge in edges {
                    if let Some(track) = self.parse_track_data(&edge["node"]) {
                        tracks.push(track);
                    }
                }
            }

            if lookup[query_type]["pageInfo"]["hasNextPage"].as_bool() == Some(true) {
                cursor = lookup[query_type]["pageInfo"]["endCursor"]
                    .as_str()
                    .map(|s| s.to_string());
                if cursor.is_none() || tracks.len() >= 1000 {
                    break;
                }
            } else {
                break;
            }
        }

        if tracks.is_empty() {
            return LoadResult::Empty {};
        }

        LoadResult::Playlist(PlaylistData {
            info: PlaylistInfo {
                name: format!("{} ({})", display_name, list_type),
                selected_track: -1,
            },
            plugin_info: json!({}),
            tracks,
        })
    }

    async fn search(&self, query_raw: &str) -> LoadResult {
        let url = format!(
            "https://api.mixcloud.com/search/?q={}&type=cloudcast",
            urlencoding::encode(query_raw)
        );
        let resp = match self.client.get(url).send().await {
            Ok(r) => r,
            Err(_) => return LoadResult::Empty {},
        };

        let body: Value = match resp.json().await {
            Ok(v) => v,
            Err(_) => return LoadResult::Empty {},
        };

        let mut tracks = Vec::new();
        if let Some(data) = body["data"].as_array() {
            for item in data {
                if let Some(url_raw) = item["url"].as_str() {
                    let path_parts: Vec<&str> = url_raw
                        .split("mixcloud.com/")
                        .nth(1)
                        .unwrap_or("")
                        .split('/')
                        .filter(|s| !s.is_empty())
                        .collect();
                    if path_parts.len() < 2 {
                        continue;
                    }

                    let id = format!("{}_{}", path_parts[0], path_parts[1]);
                    let track_info = TrackInfo {
                        identifier: id,
                        is_seekable: true,
                        author: item["user"]["name"]
                            .as_str()
                            .or(Some(path_parts[0]))
                            .unwrap()
                            .to_string(),
                        length: item["audio_length"].as_u64().unwrap_or(0) * 1000,
                        is_stream: false,
                        position: 0,
                        title: item["name"].as_str().unwrap_or("Unknown").to_string(),
                        uri: Some(url_raw.to_string()),
                        artwork_url: item["pictures"]["large"]
                            .as_str()
                            .or(item["pictures"]["medium"].as_str())
                            .map(|s| s.to_string()),
                        isrc: None,
                        source_name: "mixcloud".to_string(),
                    };
                    tracks.push(Track::new(track_info));
                }

                if tracks.len() >= self.search_limit {
                    break;
                }
            }
        }

        if tracks.is_empty() {
            return LoadResult::Empty {};
        }

        LoadResult::Search(tracks)
    }
}

pub async fn fetch_track_stream_info(client: &reqwest::Client, url: &str) -> Option<(Option<String>, Option<String>)> {
        let path_parts: Vec<&str> = url
            .split("mixcloud.com/")
            .nth(1)?
            .split('/')
            .filter(|s| !s.is_empty())
            .collect();

        if path_parts.len() < 2 {
            return None;
        }

        let query = format!(
            "{{
        cloudcastLookup(lookup: {{username: \"{}\", slug: \"{}\"}}) {{
          streamInfo {{ hlsUrl url }}
        }}
      }}",
            path_parts[0], path_parts[1]
        );

        let body = graphql_request_internal(client, &query).await?;
        let data = body["data"]["cloudcastLookup"].as_object()?;
        
        let hls = data.get("streamInfo")?.get("hlsUrl")?.as_str().map(|s| s.to_string());
        let stream = data.get("streamInfo")?.get("url")?.as_str().map(|s| s.to_string());

        Some((hls, stream))
    }

#[async_trait]
impl SourcePlugin for MixcloudSource {
    fn name(&self) -> &str {
        "mixcloud"
    }

    fn can_handle(&self, identifier: &str) -> bool {
        self.search_prefixes.iter().any(|p| identifier.starts_with(p))
            || self.track_url_re.is_match(identifier)
            || self.playlist_url_re.is_match(identifier)
            || self.user_url_re.is_match(identifier)
    }

    fn search_prefixes(&self) -> Vec<&str> {
        self.search_prefixes.iter().map(|s| s.as_str()).collect()
    }

    async fn load(
        &self,
        identifier: &str,
        _routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> LoadResult {
        for prefix in &self.search_prefixes {
            if identifier.starts_with(prefix) {
                return self.search(&identifier[prefix.len()..]).await;
            }
        }

        if let Some(caps) = self.playlist_url_re.captures(identifier) {
            return self
                .resolve_playlist(&caps["user"], &caps["playlist"])
                .await;
        }

        if let Some(caps) = self.user_url_re.captures(identifier) {
            return self
                .resolve_user(
                    &caps["id"],
                    caps.name("type").map(|m| m.as_str()).unwrap_or("uploads"),
                )
                .await;
        }

        if let Some(caps) = self.track_url_re.captures(identifier) {
            // Because this is checked last, we assume it's a track (not 'uploads', etc.)
            return self
                .resolve_track(&caps["user"], &caps["slug"])
                .await;
        }

        LoadResult::Empty {}
    }

    async fn get_track(
        &self,
        identifier: &str,
        routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<BoxedTrack> {
        let url = match self.load(identifier, None).await {
            LoadResult::Track(track) => track.info.uri?,
            _ => return None,
        };

        let (enc_hls, enc_url) = fetch_track_stream_info(&self.client, &url).await.unwrap_or((None, None));
        
        let hls_url = enc_hls.map(|s| decrypt(&s));
        let stream_url = enc_url.map(|s| decrypt(&s));

        if hls_url.is_none() && stream_url.is_none() {
            return None;
        }

        Some(Box::new(track::MixcloudTrack {
            client: self.client.clone(),
            hls_url,
            stream_url,
            uri: url,
            local_addr: routeplanner.and_then(|rp| rp.get_address()),
        }))
    }
}

async fn graphql_request_internal(client: &reqwest::Client, query: &str) -> Option<Value> {
    let url = format!("{}?query={}", GRAPHQL_URL, urlencoding::encode(query));
    let resp = client.get(url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.json::<Value>().await.ok()
}
