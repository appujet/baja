use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use super::{
    YouTubeClient,
    common::{extract_thumbnail, is_duration, parse_duration},
};
use crate::{
    common::types::AnyResult,
    protocol::tracks::{Track, TrackInfo},
    sources::youtube::{
        cipher::YouTubeCipherManager, clients::common::ClientConfig, oauth::YouTubeOAuth,
    },
};

const CLIENT_NAME: &str = "ANDROID_MUSIC";
const CLIENT_VERSION: &str = "8.47.54";
const USER_AGENT: &str =
    "com.google.android.apps.youtube.music/8.47.54 (Linux; U; Android 14 gzip)";

const INNERTUBE_API: &str = "https://music.youtube.com";

pub struct MusicAndroidClient {
    http: Arc<reqwest::Client>,
}

impl MusicAndroidClient {
    pub fn new(http: Arc<reqwest::Client>) -> Self {
        Self { http }
    }

    fn config(&self) -> ClientConfig<'_> {
        ClientConfig {
            client_name: CLIENT_NAME,
            client_version: CLIENT_VERSION,
            client_id: "67",
            user_agent: USER_AGENT,
            device_make: Some("Google"),
            device_model: Some("Pixel 6"),
            os_name: Some("Android"),
            os_version: Some("14"),
            android_sdk_version: Some("30"),
            ..Default::default()
        }
    }

    async fn player_request(
        &self,
        video_id: &str,
        visitor_data: Option<&str>,
        signature_timestamp: Option<u32>,
        _oauth: &Arc<YouTubeOAuth>,
    ) -> AnyResult<Value> {
        crate::sources::youtube::clients::common::make_player_request(
            &self.http,
            &self.config(),
            video_id,
            None,
            visitor_data,
            signature_timestamp,
            None,
            None,
            Some(INNERTUBE_API),
            None,
        )
        .await
    }
}

#[async_trait]
impl YouTubeClient for MusicAndroidClient {
    fn name(&self) -> &str {
        "MusicAndroid"
    }
    fn client_name(&self) -> &str {
        CLIENT_NAME
    }
    fn client_version(&self) -> &str {
        CLIENT_VERSION
    }
    fn user_agent(&self) -> &str {
        USER_AGENT
    }

    async fn search(
        &self,
        query: &str,
        context: &Value,
        oauth: Arc<YouTubeOAuth>,
    ) -> AnyResult<Vec<Track>> {
        let visitor_data = context
            .get("client")
            .and_then(|c| c.get("visitorData"))
            .and_then(|v| v.as_str())
            .or_else(|| context.get("visitorData").and_then(|v| v.as_str()));

        let body = json!({
            "context": self.config().build_context(None),
            "query": query,
            "params": "EgWKAQIIAWoQEAMQBBAJEAoQBRAREBAQFQ%3D%3D"
        });

        let url = format!("{}/youtubei/v1/search?prettyPrint=false", INNERTUBE_API);

        let mut req = self
            .http
            .post(&url)
            .header("X-Goog-Api-Format-Version", "2")
            .header("User-Agent", USER_AGENT);

        if let Some(vd) = visitor_data {
            req = req.header("X-Goog-Visitor-Id", vd);
        }

        let req = req.json(&body);

        let _ = oauth;

        let res = req.send().await?;
        if !res.status().is_success() {
            let status = res.status();
            let err_body = res.text().await.unwrap_or_default();
            return Err(format!("Music Android search failed: {} - {}", status, err_body).into());
        }

        let response: Value = res.json().await.unwrap_or_default();
        let mut tracks = Vec::new();

        let tab_content = response
            .get("contents")
            .and_then(|c| c.get("tabbedSearchResultsRenderer"))
            .and_then(|t| t.get("tabs"))
            .and_then(|t| t.get(0))
            .and_then(|t| t.get("tabRenderer"))
            .and_then(|t| t.get("content"));

        let mut videos = None;

        fn find_shelf(contents: &Value) -> Option<&Vec<Value>> {
            if let Some(sections) = contents.as_array() {
                for section in sections {
                    if let Some(shelf) = section.get("musicShelfRenderer") {
                        return shelf.get("contents").and_then(|c| c.as_array());
                    }
                }
            }
            None
        }

        if let Some(tab) = tab_content {
            if let Some(section_list) = tab.get("sectionListRenderer") {
                if let Some(contents) = section_list.get("contents") {
                    videos = find_shelf(contents);
                }
            }

            if videos.is_none() {
                if let Some(split_view) = tab.get("musicSplitViewRenderer") {
                    if let Some(main_content) = split_view.get("mainContent") {
                        if let Some(section_list) = main_content.get("sectionListRenderer") {
                            if let Some(contents) = section_list.get("contents") {
                                videos = find_shelf(contents);
                            }
                        }
                    }
                }
            }
        }

        if let Some(items) = videos {
            for item in items {
                let renderer = item
                    .get("musicResponsiveListItemRenderer")
                    .or_else(|| item.get("musicTwoColumnItemRenderer"))
                    .or_else(|| {
                        if item.get("videoId").is_some() {
                            Some(item)
                        } else {
                            None
                        }
                    });

                if let Some(renderer) = renderer {
                    let id = renderer
                        .get("playlistItemData")
                        .and_then(|d| d.get("videoId"))
                        .and_then(|v| v.as_str())
                        .or_else(|| {
                            renderer
                                .get("navigationEndpoint")
                                .and_then(|n| n.get("watchEndpoint"))
                                .and_then(|w| w.get("videoId"))
                                .and_then(|v| v.as_str())
                        })
                        .or_else(|| {
                            renderer
                                .get("doubleTapCommand")
                                .and_then(|c| c.get("watchEndpoint"))
                                .and_then(|w| w.get("videoId"))
                                .and_then(|v| v.as_str())
                        })
                        .or_else(|| renderer.get("videoId").and_then(|v| v.as_str()));

                    if let Some(id) = id {
                        // Title extraction
                        let mut title = renderer
                            .get("title")
                            .and_then(|t| t.get("runs"))
                            .and_then(|r| r.get(0))
                            .and_then(|r| r.get("text"))
                            .and_then(|t| t.as_str())
                            .or_else(|| {
                                renderer
                                    .get("title")
                                    .and_then(|t| t.get("simpleText"))
                                    .and_then(|t| t.as_str())
                            })
                            .or_else(|| renderer.get("title").and_then(|t| t.as_str()))
                            .unwrap_or("Unknown Title");

                        if title == "Unknown Title" {
                            if let Some(flex_cols) =
                                renderer.get("flexColumns").and_then(|c| c.as_array())
                            {
                                if !flex_cols.is_empty() {
                                    if let Some(t) = flex_cols[0]
                                        .get("musicResponsiveListItemFlexColumnRenderer")
                                        .and_then(|r| r.get("text"))
                                        .and_then(|t| t.get("runs"))
                                        .and_then(|r| r.get(0))
                                        .and_then(|r| r.get("text"))
                                        .and_then(|t| t.as_str())
                                    {
                                        title = t;
                                    }
                                }
                            }
                        }

                        // Author extraction
                        let mut author = "Unknown Artist".to_string();
                        let subtitle_runs = renderer
                            .get("subtitle")
                            .and_then(|s| s.get("runs"))
                            .and_then(|r| r.as_array());
                        let long_byline_runs = renderer
                            .get("longBylineText")
                            .and_then(|l| l.get("runs"))
                            .and_then(|r| r.as_array());
                        let short_byline_runs = renderer
                            .get("shortBylineText")
                            .and_then(|s| s.get("runs"))
                            .and_then(|r| r.as_array());

                        if let Some(runs) = subtitle_runs {
                            if !runs.is_empty() {
                                if let Some(a) = runs[0].get("text").and_then(|t| t.as_str()) {
                                    author = a.to_string();
                                }
                            }
                        } else if let Some(runs) = long_byline_runs {
                            if !runs.is_empty() {
                                if let Some(a) = runs[0].get("text").and_then(|t| t.as_str()) {
                                    author = a.to_string();
                                }
                            }
                        } else if let Some(runs) = short_byline_runs {
                            if !runs.is_empty() {
                                if let Some(a) = runs[0].get("text").and_then(|t| t.as_str()) {
                                    author = a.to_string();
                                }
                            }
                        } else if let Some(a) = renderer.get("author").and_then(|a| a.as_str()) {
                            author = a.to_string();
                        }

                        if author == "Unknown Artist" {
                            if let Some(flex_cols) =
                                renderer.get("flexColumns").and_then(|c| c.as_array())
                            {
                                if flex_cols.len() > 1 {
                                    if let Some(a) = flex_cols[1]
                                        .get("musicResponsiveListItemFlexColumnRenderer")
                                        .and_then(|r| r.get("text"))
                                        .and_then(|t| t.get("runs"))
                                        .and_then(|r| r.get(0))
                                        .and_then(|r| r.get("text"))
                                        .and_then(|t| t.as_str())
                                    {
                                        author = a.to_string();
                                    }
                                }
                            }
                        }

                        // Duration extraction
                        let mut length_ms = 0u64;
                        if let Some(runs) = subtitle_runs {
                            for run in runs {
                                if let Some(text) = run.get("text").and_then(|t| t.as_str()) {
                                    if is_duration(text) {
                                        length_ms = parse_duration(text);
                                        break;
                                    }
                                }
                            }
                        }

                        if length_ms == 0 {
                            if let Some(text) = renderer
                                .get("lengthText")
                                .and_then(|l| l.get("simpleText"))
                                .and_then(|t| t.as_str())
                            {
                                if is_duration(text) {
                                    length_ms = parse_duration(text);
                                }
                            }
                        }

                        if length_ms == 0 {
                            if let Some(runs) = renderer
                                .get("lengthText")
                                .and_then(|l| l.get("runs"))
                                .and_then(|r| r.as_array())
                            {
                                for run in runs {
                                    if let Some(text) = run.get("text").and_then(|t| t.as_str()) {
                                        if is_duration(text) {
                                            length_ms = parse_duration(text);
                                            break;
                                        }
                                    }
                                }
                            }
                        }

                        if length_ms == 0 {
                            if let Some(flex_cols) =
                                renderer.get("flexColumns").and_then(|c| c.as_array())
                            {
                                for column in flex_cols {
                                    if let Some(runs) = column
                                        .get("musicResponsiveListItemFlexColumnRenderer")
                                        .and_then(|r| r.get("text"))
                                        .and_then(|t| t.get("runs"))
                                        .and_then(|r| r.as_array())
                                    {
                                        for run in runs {
                                            if let Some(text) =
                                                run.get("text").and_then(|t| t.as_str())
                                            {
                                                if is_duration(text) {
                                                    length_ms = parse_duration(text);
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                    if length_ms > 0 {
                                        break;
                                    }
                                }
                            }
                        }

                        let artwork_url = extract_thumbnail(renderer, Some(id));

                        let info = TrackInfo {
                            identifier: id.to_string(),
                            is_seekable: true,
                            title: title.to_string(),
                            author,
                            length: length_ms,
                            is_stream: false,
                            uri: Some(format!("https://music.youtube.com/watch?v={}", id)),
                            source_name: "youtube".to_string(),
                            isrc: None,
                            artwork_url,
                            position: 0,
                        };
                        tracks.push(Track::new(info));
                    }
                }
            }
        }

        Ok(tracks)
    }

    async fn get_track_info(
        &self,
        track_id: &str,
        context: &Value,
        oauth: Arc<YouTubeOAuth>,
    ) -> AnyResult<Option<Track>> {
        let visitor_data = context
            .get("client")
            .and_then(|c| c.get("visitorData"))
            .and_then(|v| v.as_str())
            .or_else(|| context.get("visitorData").and_then(|v| v.as_str()));
        let body = self
            .player_request(track_id, visitor_data, None, &oauth)
            .await?;

        let playability = body
            .get("playabilityStatus")
            .and_then(|p| p.get("status"))
            .and_then(|s| s.as_str())
            .unwrap_or("UNKNOWN");
        if playability != "OK" {
            return Ok(None);
        }

        let vd = body.get("videoDetails");
        let title = vd
            .and_then(|v| v.get("title"))
            .and_then(|t| t.as_str())
            .unwrap_or("Unknown Title");
        let author = vd
            .and_then(|v| v.get("author"))
            .and_then(|a| a.as_str())
            .unwrap_or("Unknown Artist");
        let length_secs = vd
            .and_then(|v| v.get("lengthSeconds"))
            .and_then(|l| l.as_str())
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);

        let info = TrackInfo {
            identifier: track_id.to_string(),
            is_seekable: true,
            title: title.to_string(),
            author: author.to_string(),
            length: length_secs * 1000,
            is_stream: false,
            uri: Some(format!("https://music.youtube.com/watch?v={}", track_id)),
            source_name: "youtube".to_string(),
            isrc: None,
            artwork_url: extract_thumbnail(&vd.cloned().unwrap_or(Value::Null), Some(track_id)),
            position: 0,
        };

        Ok(Some(Track::new(info)))
    }

    async fn get_playlist(
        &self,
        playlist_id: &str,
        context: &Value,
        oauth: Arc<YouTubeOAuth>,
    ) -> AnyResult<Option<(Vec<Track>, String)>> {
        let visitor_data = context
            .get("client")
            .and_then(|c| c.get("visitorData"))
            .and_then(|v| v.as_str())
            .or_else(|| context.get("visitorData").and_then(|v| v.as_str()));

        let body = json!({
            "context": self.config().build_context(visitor_data),
            "playlistId": playlist_id,
            "enablePersistentPlaylistPanel": true,
            "isAudioOnly": true
        });

        let url = format!("{}/youtubei/v1/next?prettyPrint=false", INNERTUBE_API);

        let mut req = self
            .http
            .post(&url)
            .header("User-Agent", USER_AGENT)
            .header("X-YouTube-Client-Name", "67")
            .header("X-YouTube-Client-Version", CLIENT_VERSION);

        if let Some(vd) = visitor_data {
            req = req.header("X-Goog-Visitor-Id", vd);
        }

        let req = req.json(&body);

        let _ = oauth;

        let res = req.send().await?;
        if !res.status().is_success() {
            return Ok(None);
        }

        let body: Value = res.json().await?;
        let result = crate::sources::youtube::extractor::extract_from_next(&body, "youtube");

        if result.is_none() {
            tracing::warn!("MusicAndroid: extract_from_next returned None");
            if let Some(obj) = body.as_object() {
                tracing::warn!("Response keys: {:?}", obj.keys());
            }
            if let Some(contents) = body.get("contents") {
                if let Some(obj) = contents.as_object() {
                    tracing::warn!("Contents keys: {:?}", obj.keys());
                    if let Some(renderer) = obj.get("singleColumnMusicWatchNextResultsRenderer") {
                        tracing::warn!(
                            "Renderer keys: {:?}",
                            renderer.as_object().map(|o| o.keys())
                        );
                    }
                }
            } else {
                tracing::warn!("No 'contents' field in response");
            }
        }

        Ok(result)
    }

    async fn resolve_url(
        &self,
        _url: &str,
        _context: &Value,
        _oauth: Arc<YouTubeOAuth>,
    ) -> AnyResult<Option<Track>> {
        Ok(None)
    }

    async fn get_track_url(
        &self,
        _track_id: &str,
        _context: &Value,
        _cipher_manager: Arc<YouTubeCipherManager>,
        _oauth: Arc<YouTubeOAuth>,
    ) -> AnyResult<Option<String>> {
        tracing::debug!("{} client does not provide direct track URLs", self.name());
        Ok(None)
    }

    async fn get_player_body(
        &self,
        track_id: &str,
        visitor_data: Option<&str>,
        oauth: Arc<YouTubeOAuth>,
    ) -> Option<serde_json::Value> {
        self.player_request(track_id, visitor_data, None, &oauth)
            .await
            .ok()
    }
}
