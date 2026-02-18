use crate::api::tracks::{LoadResult, PlaylistData, PlaylistInfo, Track, TrackInfo};
use crate::sources::SourcePlugin;
use async_trait::async_trait;
use regex::Regex;
use reqwest::header::{COOKIE, HeaderMap, HeaderValue, USER_AGENT};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, warn};

const EMBED_URL: &str = "https://open.spotify.com/embed/track/4cOdK2wGLETKBW3PvgPWqT";
const PARTNER_API_URL: &str = "https://api-partner.spotify.com/pathfinder/v2/query";

#[derive(Clone, Debug)]
struct SpotifyToken {
    access_token: String,
    expiry_ms: u64,
}

pub struct SpotifySource {
    client: reqwest::Client,
    search_prefix: String,
    url_regex: Regex,
    track_regex: Regex,
    album_regex: Regex,
    playlist_regex: Regex,
    artist_regex: Regex,
    sp_dc: Option<String>,
    token: Arc<RwLock<Option<SpotifyToken>>>,
}

impl SpotifySource {
    pub fn new(config: Option<crate::config::SpotifyConfig>) -> Self {
        let mut headers = HeaderMap::new();
        headers.insert(
            USER_AGENT,
            HeaderValue::from_static("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        );

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .build()
            .unwrap();

        Self {
            client,
            search_prefix: "spsearch:".to_string(),
            url_regex: Regex::new(r"https?://(?:open\.)?spotify\.com/(?:intl-[a-z]{2}/)?(track|album|playlist|artist)/([a-zA-Z0-9]+)").unwrap(),
            track_regex: Regex::new(r"https?://(?:open\.)?spotify\.com/(?:intl-[a-z]{2}/)?track/([a-zA-Z0-9]+)").unwrap(),
            album_regex: Regex::new(r"https?://(?:open\.)?spotify\.com/(?:intl-[a-z]{2}/)?album/([a-zA-Z0-9]+)").unwrap(),
            playlist_regex: Regex::new(r"https?://(?:open\.)?spotify\.com/(?:intl-[a-z]{2}/)?playlist/([a-zA-Z0-9]+)").unwrap(),
            artist_regex: Regex::new(r"https?://(?:open\.)?spotify\.com/(?:intl-[a-z]{2}/)?artist/([a-zA-Z0-9]+)").unwrap(),
            sp_dc: config.and_then(|c| c.sp_dc),
            token: Arc::new(RwLock::new(None)),
        }
    }

    async fn get_token(&self) -> Option<String> {
        {
            let token_lock = self.token.read().await;
            if let Some(token) = &*token_lock {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64;
                if token.expiry_ms > now + 60000 {
                    return Some(token.access_token.clone());
                }
            }
        }

        self.refresh_token().await
    }

    async fn refresh_token(&self) -> Option<String> {
        debug!("Refreshing Spotify token from embed...");
        let mut request = self
            .client
            .get(EMBED_URL)
            .header("Accept-Language", "en-US,en;q=0.9")
            .header("Sec-Fetch-Dest", "iframe")
            .header("Sec-Fetch-Mode", "navigate")
            .header("Sec-Fetch-Site", "cross-site");

        if let Some(sp_dc) = &self.sp_dc {
            if !sp_dc.trim().is_empty() {
                request = request.header(COOKIE, format!("sp_dc={}", sp_dc));
            }
        }

        let resp = match request.send().await {
            Ok(r) => r,
            Err(e) => {
                error!("Failed to fetch Spotify embed page: {}", e);
                return None;
            }
        };

        if !resp.status().is_success() {
            error!("Embed page returned status {}", resp.status());
            return None;
        }

        let html = match resp.text().await {
            Ok(t) => t,
            Err(e) => {
                error!("Failed to read Spotify embed HTML: {}", e);
                return None;
            }
        };

        let token_regex = Regex::new(r#""accessToken":"([^"]+)""#).unwrap();
        let expiry_regex = Regex::new(r#""accessTokenExpirationTimestampMs":(\d+)"#).unwrap();

        let token_caps = token_regex.captures(&html);
        let expiry_caps = expiry_regex.captures(&html);

        if token_caps.is_none() || expiry_caps.is_none() {
            error!("Token or expiry not found in embed page");
            return None;
        }

        let token = token_caps.unwrap().get(1).unwrap().as_str().to_string();
        let expiry_ms = expiry_caps
            .unwrap()
            .get(1)
            .unwrap()
            .as_str()
            .parse::<u64>()
            .ok()?;

        let mut token_lock = self.token.write().await;
        *token_lock = Some(SpotifyToken {
            access_token: token.clone(),
            expiry_ms,
        });

        debug!(
            "Successfully refreshed Spotify token. Expiry: {}",
            expiry_ms
        );
        Some(token)
    }

    async fn partner_api_request(
        &self,
        operation: &str,
        variables: Value,
        sha256_hash: &str,
    ) -> Option<Value> {
        let token = self.get_token().await?;

        let body = json!({
            "variables": variables,
            "operationName": operation,
            "extensions": {
                "persistedQuery": {
                    "version": 1,
                    "sha256Hash": sha256_hash
                }
            }
        });

        let resp = self
            .client
            .post(PARTNER_API_URL)
            .bearer_auth(token)
            .header("App-Platform", "WebPlayer")
            .header("Spotify-App-Version", "1.2.81.104.g225ec0e6")
            .json(&body)
            .send()
            .await
            .ok()?;

        if !resp.status().is_success() {
            warn!("Partner API returned {} for {}", resp.status(), operation);
            return None;
        }

        resp.json().await.ok()
    }

    async fn search_internal(&self, query: &str) -> LoadResult {
        let variables = json!({
            "searchTerm": query,
            "offset": 0,
            "limit": 10,
            "numberOfTopResults": 5,
            "includeAudiobooks": false,
            "includeArtistHasConcertsField": false,
            "includePreReleases": false
        });

        let hash = "fcad5a3e0d5af727fb76966f06971c19cfa2275e6ff7671196753e008611873c";

        let data = match self
            .partner_api_request("searchDesktop", variables, hash)
            .await
        {
            Some(d) => d,
            None => return LoadResult::Empty {},
        };

        let mut tracks = Vec::new();

        if let Some(items) = data
            .pointer("/data/searchV2/tracksV2/items")
            .and_then(|v| v.as_array())
        {
            for item in items {
                if let Some(track_data) = item.get("item").and_then(|v| v.get("data")) {
                    if let Some(mut track_info) = self.parse_generic_track(track_data, None) {
                        // Fetch ISRC for top results if missing to improve mirroring
                        if tracks.is_empty() && track_info.isrc.is_none() {
                            if let Some(full_info) = self.fetch_track(&track_info.identifier).await
                            {
                                track_info.isrc = full_info.isrc;
                            }
                        }
                        tracks.push(Track::new(track_info));
                    }
                }
            }
        }

        if tracks.is_empty() {
            LoadResult::Empty {}
        } else {
            LoadResult::Search(tracks)
        }
    }

    fn parse_generic_track(&self, track: &Value, artwork_url: Option<String>) -> Option<TrackInfo> {
        let uri = track.get("uri")?.as_str()?;
        let id = uri.split(':').last()?;
        let title = track.get("name")?.as_str()?.to_string();

        let author = if let Some(artists) = track
            .get("artists")
            .and_then(|a| a.get("items"))
            .and_then(|i| i.as_array())
        {
            artists
                .iter()
                .filter_map(|a| a.get("profile")?.get("name")?.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        } else if let Some(artists) = track.get("artists").and_then(|a| a.as_array()) {
            // Some responses have a direct array of artists
            artists
                .iter()
                .filter_map(|a| a.get("name")?.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        } else {
            "Unknown".to_string()
        };

        let length = track.get("duration")?.get("totalMilliseconds")?.as_u64()?;

        let final_artwork = artwork_url.or_else(|| {
            track
                .get("albumOfTrack")?
                .get("coverArt")?
                .get("sources")?
                .as_array()?
                .first()?
                .get("url")?
                .as_str()?
                .to_string()
                .into()
        });

        let isrc = track
            .get("externalIds")
            .and_then(|ids| ids.get("items"))
            .and_then(|items| items.as_array())
            .and_then(|items| {
                items
                    .iter()
                    .find(|i| i.get("type").and_then(|v| v.as_str()) == Some("isrc"))
            })
            .and_then(|i| i.get("id"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        Some(TrackInfo {
            title,
            author,
            length,
            identifier: id.to_string(),
            is_stream: false,
            uri: Some(format!("https://open.spotify.com/track/{}", id)),
            artwork_url: final_artwork,
            isrc,
            source_name: "spotify".to_string(),
            is_seekable: true,
            position: 0,
        })
    }

    async fn fetch_track(&self, id: &str) -> Option<TrackInfo> {
        let variables = json!({
            "uri": format!("spotify:track:{}", id)
        });

        let hash = "612585ae06ba435ad26369870deaae23b5c8800a256cd8a57e08eddc25a37294";

        let data = self
            .partner_api_request("getTrack", variables, hash)
            .await?;
        let track = data.pointer("/data/trackUnion")?;
        self.parse_generic_track(track, None)
    }

    async fn fetch_album(&self, id: &str) -> LoadResult {
        let variables = json!({
            "uri": format!("spotify:album:{}", id),
            "locale": "en",
            "offset": 0,
            "limit": 50
        });

        let hash = "b9bfabef66ed756e5e13f68a942deb60bd4125ec1f1be8cc42769dc0259b4b10";
        let data = match self.partner_api_request("getAlbum", variables, hash).await {
            Some(d) => d,
            None => return LoadResult::Empty {},
        };

        let album = match data.pointer("/data/albumUnion") {
            Some(a) => a,
            None => return LoadResult::Empty {},
        };

        let name = album
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown Album")
            .to_string();
        let artwork_url = album
            .get("coverArt")
            .and_then(|c| c.get("sources"))
            .and_then(|s| s.as_array())
            .and_then(|s| s.first())
            .and_then(|i| i.get("url"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let mut tracks = Vec::new();
        if let Some(items) = album.pointer("/tracksV2/items").and_then(|i| i.as_array()) {
            for item in items {
                if let Some(track_data) = item.get("track") {
                    if let Some(track_info) =
                        self.parse_generic_track(track_data, artwork_url.clone())
                    {
                        tracks.push(Track::new(track_info));
                    }
                }
            }
        }

        if tracks.is_empty() {
            LoadResult::Empty {}
        } else {
            LoadResult::Playlist(PlaylistData {
                info: PlaylistInfo {
                    name,
                    selected_track: -1,
                },
                plugin_info: json!({}),
                tracks,
            })
        }
    }

    async fn fetch_playlist(&self, id: &str) -> LoadResult {
        let variables = json!({
            "uri": format!("spotify:playlist:{}", id),
            "offset": 0,
            "limit": 100
        });

        let hash = "bb67e0af06e8d6f52b531f97468ee4acd44cd0f82b988e15c2ea47b1148efc77";
        let data = match self
            .partner_api_request("fetchPlaylist", variables, hash)
            .await
        {
            Some(d) => d,
            None => return LoadResult::Empty {},
        };

        let playlist = match data.pointer("/data/playlistV2") {
            Some(p) => p,
            None => return LoadResult::Empty {},
        };

        let name = playlist
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown Playlist")
            .to_string();

        // Artwork for playlist might be in attributes or images
        let artwork_url = playlist
            .pointer("/attributes/image/sources")
            .and_then(|s| s.as_array())
            .and_then(|s| s.first())
            .or_else(|| {
                playlist
                    .get("images")
                    .and_then(|i| i.get("items"))
                    .and_then(|i| i.as_array())
                    .and_then(|i| i.first())
                    .and_then(|i| i.get("sources"))
                    .and_then(|s| s.as_array())
                    .and_then(|s| s.first())
            })
            .and_then(|i| i.get("url"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let mut tracks = Vec::new();
        if let Some(items) = playlist
            .pointer("/content/items")
            .and_then(|i| i.as_array())
        {
            for item in items {
                if let Some(item_data) = item.get("item").and_then(|v| v.get("data")) {
                    // Check if it's a track
                    if item_data.get("__typename").and_then(|v| v.as_str()) == Some("Track") {
                        if let Some(track_info) =
                            self.parse_generic_track(item_data, artwork_url.clone())
                        {
                            tracks.push(Track::new(track_info));
                        }
                    }
                }
            }
        }

        if tracks.is_empty() {
            LoadResult::Empty {}
        } else {
            LoadResult::Playlist(PlaylistData {
                info: PlaylistInfo {
                    name,
                    selected_track: -1,
                },
                plugin_info: json!({}),
                tracks,
            })
        }
    }

    async fn fetch_artist_top_tracks(&self, id: &str) -> LoadResult {
        let variables = json!({
            "uri": format!("spotify:artist:{}", id),
            "locale": "en",
            "includeUpcoming": false
        });

        let hash = "35648a112beb1794e39ab931365f6ae4a8d45e65396d641eeda94e4003d41497";
        let data = match self
            .partner_api_request("queryArtistOverview", variables, hash)
            .await
        {
            Some(d) => d,
            None => return LoadResult::Empty {},
        };

        let artist = match data.pointer("/data/artistUnion") {
            Some(a) => a,
            None => return LoadResult::Empty {},
        };

        let name = format!(
            "{} Top Tracks",
            artist
                .get("profile")
                .and_then(|p| p.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown Artist")
        );

        let mut tracks = Vec::new();
        if let Some(top_tracks) = artist
            .pointer("/discography/topTracks/items")
            .and_then(|i| i.as_array())
        {
            for item in top_tracks {
                if let Some(track_data) = item.get("track") {
                    if let Some(track_info) = self.parse_generic_track(track_data, None) {
                        tracks.push(Track::new(track_info));
                    }
                }
            }
        }

        if tracks.is_empty() {
            LoadResult::Empty {}
        } else {
            LoadResult::Playlist(PlaylistData {
                info: PlaylistInfo {
                    name,
                    selected_track: -1,
                },
                plugin_info: json!({}),
                tracks,
            })
        }
    }
}

#[async_trait]
impl SourcePlugin for SpotifySource {
    fn name(&self) -> &str {
        "spotify"
    }

    fn can_handle(&self, identifier: &str) -> bool {
        identifier.starts_with(&self.search_prefix) || self.url_regex.is_match(identifier)
    }

    async fn load(
        &self,
        identifier: &str,
        _routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> LoadResult {
        if identifier.starts_with(&self.search_prefix) {
            let query = identifier.strip_prefix(&self.search_prefix).unwrap();
            return self.search_internal(query).await;
        }

        if let Some(caps) = self.track_regex.captures(identifier) {
            let id = caps.get(1).unwrap().as_str();
            if let Some(track_info) = self.fetch_track(id).await {
                return LoadResult::Track(Track::new(track_info));
            }
        }

        if let Some(caps) = self.album_regex.captures(identifier) {
            let id = caps.get(1).unwrap().as_str();
            return self.fetch_album(id).await;
        }

        if let Some(caps) = self.playlist_regex.captures(identifier) {
            let id = caps.get(1).unwrap().as_str();
            return self.fetch_playlist(id).await;
        }

        if let Some(caps) = self.artist_regex.captures(identifier) {
            let id = caps.get(1).unwrap().as_str();
            return self.fetch_artist_top_tracks(id).await;
        }

        LoadResult::Empty {}
    }

    async fn get_playback_url(
        &self,
        _identifier: &str,
        _routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
    ) -> Option<String> {
        None
    }
}
