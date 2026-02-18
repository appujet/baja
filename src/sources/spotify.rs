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

    fn base62_to_hex(&self, id: &str) -> String {
        let alphabet = "0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
        let mut bn = 0u128;
        for c in id.chars() {
            if let Some(idx) = alphabet.find(c) {
                // Handle potential overflow for 131-bit numbers by keeping top 128 bits accurately
                // Most IDs will fit in 128 bits.
                bn = bn.wrapping_mul(62).wrapping_add(idx as u128);
            }
        }
        format!("{:032x}", bn)
    }

    async fn fetch_metadata_isrc(&self, id: &str) -> Option<String> {
        let token = self.get_token().await?;
        let hex_id = self.base62_to_hex(id);
        let url = format!(
            "https://spclient.wg.spotify.com/metadata/4/track/{}?market=from_token",
            hex_id
        );

        debug!(
            "Fetching Spotify metadata ISRC for {} (hex: {})",
            id, hex_id
        );

        let resp = self
            .client
            .get(&url)
            .bearer_auth(token)
            .header("App-Platform", "WebPlayer")
            .header("Spotify-App-Version", "1.2.81.104.g225ec0e6")
            .send()
            .await
            .ok()?;

        if !resp.status().is_success() {
            debug!("Metadata API returned {} for {}", resp.status(), id);
            return None;
        }

        let body_bytes = resp.bytes().await.ok()?;

        // Search for ISRC in binary data
        let isrc_marker = b"isrc";
        if let Some(pos) = body_bytes.windows(4).position(|w| w == isrc_marker) {
            let start = pos;
            let end = std::cmp::min(pos + 64, body_bytes.len());
            let chunk = &body_bytes[start..end];

            // Convert chunk to a lossy string for regex (safe slicing)
            let chunk_str = String::from_utf8_lossy(chunk);
            let re = Regex::new(r"[A-Z0-9]{12}").unwrap();
            if let Some(mat) = re.find(&chunk_str) {
                let isrc = mat.as_str().to_string();
                debug!("Found ISRC in metadata body via regex: {}", isrc);
                return Some(isrc);
            }
        }

        // Try to parse as JSON if the above fails (rare for this endpoint)
        if let Ok(json_str) = std::str::from_utf8(&body_bytes) {
            if let Ok(json) = serde_json::from_str::<Value>(json_str) {
                if let Some(isrc) = json
                    .get("external_id")
                    .and_then(|ids| ids.as_array())
                    .and_then(|items| {
                        items
                            .iter()
                            .find(|i| i.get("type").and_then(|v| v.as_str()) == Some("isrc"))
                    })
                    .and_then(|i| i.get("id"))
                    .and_then(|v| v.as_str())
                {
                    debug!("Found ISRC in metadata JSON: {}", isrc);
                    return Some(isrc.to_string());
                }
            }
        }

        debug!("No ISRC found in metadata for {}", id);
        None
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
                    if let Some(track_info) = self.parse_generic_track(track_data, None).await {
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

    async fn parse_generic_track(
        &self,
        track_val: &Value,
        artwork_url: Option<String>,
    ) -> Option<TrackInfo> {
        // Some structures wrap the track info in a "track" property
        let track = if track_val.get("uri").is_some() {
            track_val
        } else if let Some(inner) = track_val.get("track") {
            inner
        } else {
            debug!(
                "Track data missing uri and no nested track property: {:?}",
                track_val
            );
            return None;
        };

        let uri = match track.get("uri").and_then(|v| v.as_str()) {
            Some(u) => u,
            None => {
                debug!("Track missing uri: {:?}", track);
                return None;
            }
        };
        let id = uri.split(':').last()?;
        let title = match track.get("name").and_then(|v| v.as_str()) {
            Some(t) => t.to_string(),
            None => {
                debug!("Track missing name: {:?}", track);
                return None;
            }
        };

        let author = if let Some(artists) = track
            .get("artists")
            .and_then(|a| a.get("items"))
            .and_then(|i| i.as_array())
        {
            artists
                .iter()
                .filter_map(|a| {
                    a.get("profile")
                        .and_then(|p| p.get("name"))
                        .or_else(|| a.get("name"))
                        .and_then(|v| v.as_str())
                })
                .collect::<Vec<_>>()
                .join(", ")
        } else if let Some(first_artist) = track
            .get("firstArtist")
            .and_then(|a| a.get("items"))
            .and_then(|i| i.as_array())
            .and_then(|i| i.first())
        {
            let first_name = first_artist
                .get("profile")
                .and_then(|p| p.get("name"))
                .or_else(|| first_artist.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown");
            let mut all_artists = vec![first_name];

            if let Some(others) = track
                .get("otherArtists")
                .and_then(|a| a.get("items"))
                .and_then(|i| i.as_array())
            {
                for artist in others {
                    if let Some(name) = artist
                        .get("profile")
                        .and_then(|p| p.get("name"))
                        .or_else(|| artist.get("name"))
                        .and_then(|v| v.as_str())
                    {
                        all_artists.push(name);
                    }
                }
            }
            all_artists.join(", ")
        } else if let Some(artists) = track.get("artists").and_then(|a| a.as_array()) {
            artists
                .iter()
                .filter_map(|a| {
                    a.get("name")
                        .or_else(|| a.get("profile").and_then(|p| p.get("name")))
                        .and_then(|v| v.as_str())
                })
                .collect::<Vec<_>>()
                .join(", ")
        } else {
            track
                .get("artist")
                .and_then(|a| a.get("name"))
                .and_then(|v| v.as_str())
                .or_else(|| {
                    track
                        .get("artists")
                        .and_then(|a| a.as_array())
                        .and_then(|a| a.first())
                        .and_then(|a| a.get("name"))
                        .and_then(|v| v.as_str())
                })
                .unwrap_or("Unknown Artist")
                .to_string()
        };

        let length = track
            .get("duration")
            .or_else(|| track.get("trackDuration"))
            .and_then(|d| d.get("totalMilliseconds"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        let final_artwork = artwork_url.or_else(|| {
            track
                .get("albumOfTrack")
                .and_then(|a| a.get("coverArt"))
                .and_then(|c| c.get("sources"))
                .and_then(|s| s.as_array())
                .and_then(|s| s.first())
                .and_then(|i| i.get("url"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .or_else(|| {
                    track
                        .get("album")
                        .and_then(|a| a.get("images"))
                        .and_then(|i| i.as_array())
                        .and_then(|i| i.first())
                        .and_then(|i| i.get("url"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                })
        });

        let mut isrc = track
            .get("externalIds")
            .or_else(|| track.get("external_ids"))
            .and_then(|ids| {
                // Try direct isrc property first (common in search results)
                if let Some(isrc) = ids.get("isrc").and_then(|v| v.as_str()) {
                    return Some(isrc.to_string());
                }

                // Fallback to items list (common in fetchTrack)
                ids.get("items")
                    .and_then(|items| items.as_array())
                    .and_then(|items| {
                        items
                            .iter()
                            .find(|i| i.get("type").and_then(|v| v.as_str()) == Some("isrc"))
                    })
                    .and_then(|i| i.get("id"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            });

        // Use metadata API as fallback (common for non-official search results)
        if isrc.is_none() {
            isrc = self.fetch_metadata_isrc(id).await;
        }

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
        self.parse_generic_track(track, None).await
    }

    async fn fetch_album(&self, id: &str) -> LoadResult {
        let variables = json!({
            "uri": format!("spotify:album:{}", id),
            "locale": "en",
            "offset": 0,
            "limit": 300
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
                    if let Some(track_info) = self
                        .parse_generic_track(track_data, artwork_url.clone())
                        .await
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
            "limit": 100,
            "enableWatchFeedEntrypoint": false
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
            debug!("Playlist has {} items", items.len());
            for (i, item) in items.iter().enumerate() {
                // Handle both itemV2 (newer) and item (older) structures
                if let Some(item_data) = item
                    .get("itemV2")
                    .or_else(|| item.get("item"))
                    .and_then(|v| v.get("data"))
                {
                    // Check if it's a track
                    let typename = item_data.get("__typename").and_then(|v| v.as_str());
                    if typename == Some("Track") || typename.is_none() {
                        if let Some(track_info) = self
                            .parse_generic_track(item_data, artwork_url.clone())
                            .await
                        {
                            tracks.push(Track::new(track_info));
                        } else {
                            debug!("Failed to parse track at index {}", i);
                        }
                    } else {
                        debug!(
                            "Item at index {} is not a Track (typename: {:?})",
                            i, typename
                        );
                    }
                } else {
                    debug!("Item at index {} has no data/itemV2", i);
                }
            }
        } else {
            warn!("Playlist has no content/items");
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
            "includePrerelease": true
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
                    if let Some(track_info) = self.parse_generic_track(track_data, None).await {
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
