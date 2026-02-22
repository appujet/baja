use std::sync::Arc;

use async_trait::async_trait;
use futures::future::join_all;
use regex::Regex;
use reqwest::header::{HeaderMap, HeaderValue, USER_AGENT};
use serde_json::{Value, json};
use tokio::sync::Semaphore;
use tracing::{debug, warn};

use super::token::SpotifyTokenTracker;
use crate::{
  api::tracks::{LoadResult, PlaylistData, PlaylistInfo, Track, TrackInfo},
  sources::SourcePlugin,
};

const PARTNER_API_URL: &str = "https://api-partner.spotify.com/pathfinder/v2/query";

pub struct SpotifySource {
  client: reqwest::Client,
  search_prefixes: Vec<String>,
  rec_prefixes: Vec<String>,
  url_regex: Regex,
  track_regex: Regex,
  album_regex: Regex,
  playlist_regex: Regex,
  artist_regex: Regex,
  mix_regex: Regex,
  isrc_binary_regex: Regex,
  token_tracker: Arc<SpotifyTokenTracker>,
  // Limits
  playlist_load_limit: usize,
  album_load_limit: usize,
  search_limit: usize,
  recommendations_limit: usize,

  playlist_page_load_concurrency: usize,
  album_page_load_concurrency: usize,
  track_resolve_concurrency: usize,
}

impl SpotifySource {
  pub fn new(config: Option<crate::configs::SpotifyConfig>) -> Result<Self, String> {
    let mut headers = HeaderMap::new();
    headers.insert(
      USER_AGENT,
      HeaderValue::from_static(
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.6998.178 Spotify/1.2.65.255 Safari/537.36",
      ),
    );

    let client = reqwest::Client::builder()
      .default_headers(headers)
      .build()
      .map_err(|e| e.to_string())?;

    let (
      playlist_load_limit,
      album_load_limit,
      search_limit,
      recommendations_limit,
      playlist_page_load_concurrency,
      album_page_load_concurrency,
      track_resolve_concurrency,
    ) = if let Some(c) = config {
      (
        c.playlist_load_limit,
        c.album_load_limit,
        c.search_limit,
        c.recommendations_limit,
        c.playlist_page_load_concurrency,
        c.album_page_load_concurrency,
        c.track_resolve_concurrency,
      )
    } else {
      (6, 6, 10, 10, 10, 5, 50)
    };

    let token_tracker = Arc::new(crate::sources::spotify::token::SpotifyTokenTracker::new(
      client.clone(),
    ));
    token_tracker.clone().init();

    Ok(Self {
      client,
      search_prefixes: vec!["spsearch:".to_string()],
      rec_prefixes: vec!["sprec:".to_string()],
      url_regex: Regex::new(
        r"https?://(?:open\.)?spotify\.com/(?:intl-[a-z]{2}/)?(track|album|playlist|artist)/([a-zA-Z0-9]+)",
      )
      .unwrap(),
      track_regex: Regex::new(
        r"https?://(?:open\.)?spotify\.com/(?:intl-[a-z]{2}/)?track/([a-zA-Z0-9]+)",
      )
      .unwrap(),
      album_regex: Regex::new(
        r"https?://(?:open\.)?spotify\.com/(?:intl-[a-z]{2}/)?album/([a-zA-Z0-9]+)",
      )
      .unwrap(),
      playlist_regex: Regex::new(
        r"https?://(?:open\.)?spotify\.com/(?:intl-[a-z]{2}/)?playlist/([a-zA-Z0-9]+)",
      )
      .unwrap(),
      artist_regex: Regex::new(
        r"https?://(?:open\.)?spotify\.com/(?:intl-[a-z]{2}/)?artist/([a-zA-Z0-9]+)",
      )
      .unwrap(),
      mix_regex: Regex::new(r"mix:(album|artist|track|isrc):([a-zA-Z0-9\-_]+)").unwrap(),
      // Pre-compiled
      isrc_binary_regex: Regex::new(r"[A-Z0-9]{12}").unwrap(),
      token_tracker,
      // Limits
      playlist_load_limit,
      album_load_limit,
      search_limit,
      recommendations_limit,
      playlist_page_load_concurrency,
      album_page_load_concurrency,
      track_resolve_concurrency,
    })
  }

  fn base62_to_hex(&self, id: &str) -> String {
    const ALPHABET: &str = "0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let mut bn = 0u128;
    for c in id.chars() {
      if let Some(idx) = ALPHABET.find(c) {
        bn = bn.wrapping_mul(62).wrapping_add(idx as u128);
      }
    }
    format!("{:032x}", bn)
  }

  async fn fetch_metadata_isrc(&self, id: &str) -> Option<String> {
    let token = self.token_tracker.get_token().await?;
    let hex_id = self.base62_to_hex(id);
    let url = format!(
      "https://spclient.wg.spotify.com/metadata/4/track/{}?market=from_token",
      hex_id
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
      return None;
    }

    let body_bytes = resp.bytes().await.ok()?;

    // Fast binary scan for "isrc" marker
    let isrc_marker = b"isrc";
    if let Some(pos) = body_bytes.windows(4).position(|w| w == isrc_marker) {
      let end = std::cmp::min(pos + 64, body_bytes.len());
      let chunk_str = String::from_utf8_lossy(&body_bytes[pos..end]);
      // Use pre-compiled regex
      if let Some(mat) = self.isrc_binary_regex.find(&chunk_str) {
        return Some(mat.as_str().to_string());
      }
    }

    // JSON fallback (rare for this endpoint)
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
          return Some(isrc.to_string());
        }
      }
    }

    None
  }

  async fn partner_api_request(
    &self,
    operation: &str,
    variables: Value,
    sha256_hash: &str,
  ) -> Option<Value> {
    let token = self.token_tracker.get_token().await?;

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

  async fn fetch_paginated_items(
    &self,
    operation: &str,
    sha256_hash: &str,
    base_vars: Value,
    items_pointer: &str,
    total_count: u64,
    page_limit: u64,
    concurrency: usize,
  ) -> Vec<Value> {
    let pages_needed = total_count.saturating_sub(page_limit);
    if pages_needed == 0 {
      return Vec::new();
    }

    // Build one variables blob per remaining page
    let offsets: Vec<u64> = (1..=((total_count - 1) / page_limit)).collect();
    let semaphore = Arc::new(Semaphore::new(concurrency));

    let futs: Vec<_> = offsets
      .into_iter()
      .map(|page_idx| {
        let semaphore = semaphore.clone();
        let mut vars = base_vars.clone();
        // Patch offset in the variables object
        if let Some(obj) = vars.as_object_mut() {
          obj.insert("offset".to_string(), json!(page_idx * page_limit));
          obj.insert("limit".to_string(), json!(page_limit));
        }

        let op = operation.to_string();
        let h = sha256_hash.to_string();

        async move {
          let _permit = semaphore.acquire().await.unwrap();
          self.partner_api_request(&op, vars, &h).await
        }
      })
      .collect();

    let results = join_all(futs).await;

    results
      .into_iter()
      .flatten()
      .filter_map(|result| {
        result
          .pointer(items_pointer)
          .and_then(|v| v.as_array())
          .cloned()
      })
      .flatten()
      .collect()
  }

  async fn parse_generic_track(
    &self,
    track_val: &Value,
    artwork_url: Option<String>,
  ) -> Option<TrackInfo> {
    let track_info = self.parse_track_inner(track_val, artwork_url)?;

    // ISRC metadata fallback (expensive: one extra HTTP call per track)
    if track_info.isrc.is_none() {
      let isrc = self.fetch_metadata_isrc(&track_info.identifier).await;
      return Some(TrackInfo { isrc, ..track_info });
    }

    Some(track_info)
  }

  fn parse_track_inner(&self, track_val: &Value, artwork_url: Option<String>) -> Option<TrackInfo> {
    // Support both flat and nested track structures
    let track = if track_val.get("uri").is_some() {
      track_val
    } else if let Some(inner) = track_val.get("track") {
      inner
    } else if let Some(inner) = track_val.get("item") {
      inner
    } else if let Some(inner) = track_val.get("data") {
      inner
    } else {
      debug!(
        "Track data missing uri and no nested track property: {:?}",
        track_val
      );
      return None;
    };

    let uri = track.get("uri").and_then(|v| v.as_str())?;
    let id = uri.split(':').last()?.to_string();

    let title = track.get("name").and_then(|v| v.as_str())?.to_string();

    // Artist name resolution — handles multiple API response shapes
    let author = Self::extract_author(track);

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

    let isrc = Self::extract_isrc_inline(track);

    Some(TrackInfo {
      title,
      author,
      length,
      identifier: id.clone(),
      is_stream: false,
      uri: Some(format!("https://open.spotify.com/track/{}", id)),
      artwork_url: final_artwork,
      isrc,
      source_name: "spotify".to_string(),
      is_seekable: true,
      position: 0,
    })
  }

  /// Extract artist name(s) from any known Spotify API shape.
  fn extract_author(track: &Value) -> String {
    // Shape 1: artists.items[].profile.name  (partner API search / getTrack)
    if let Some(artists) = track
      .get("artists")
      .and_then(|a| a.get("items"))
      .and_then(|i| i.as_array())
    {
      let names: Vec<_> = artists
        .iter()
        .filter_map(|a| {
          a.get("profile")
            .and_then(|p| p.get("name"))
            .or_else(|| a.get("name"))
            .and_then(|v| v.as_str())
        })
        .collect();
      if !names.is_empty() {
        return names.join(", ");
      }
    }

    // Shape 2: firstArtist / otherArtists split
    if let Some(first_artist) = track
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
      return all_artists.join(", ");
    }

    // Shape 3: artists[] flat array (official API)
    if let Some(artists) = track.get("artists").and_then(|a| a.as_array()) {
      let names: Vec<_> = artists
        .iter()
        .filter_map(|a| {
          a.get("name")
            .or_else(|| a.get("profile").and_then(|p| p.get("name")))
            .and_then(|v| v.as_str())
        })
        .collect();
      if !names.is_empty() {
        return names.join(", ");
      }
    }

    // Fallback
    track
      .get("artist")
      .and_then(|a| a.get("name"))
      .and_then(|v| v.as_str())
      .unwrap_or("Unknown Artist")
      .to_string()
  }

  /// Extract ISRC that is already present in the response payload (no network call).
  fn extract_isrc_inline(track: &Value) -> Option<String> {
    track
      .get("externalIds")
      .or_else(|| track.get("external_ids"))
      .and_then(|ids| {
        // Direct property (common in search results)
        if let Some(isrc) = ids
          .get("isrc")
          .and_then(|v| v.as_str())
          .filter(|s| !s.is_empty())
        {
          return Some(isrc.to_string());
        }
        // Items list (common in fetchTrack)
        ids
          .get("items")
          .and_then(|items| items.as_array())
          .and_then(|items| {
            items
              .iter()
              .find(|i| i.get("type").and_then(|v| v.as_str()) == Some("isrc"))
          })
          .and_then(|i| i.get("id"))
          .and_then(|v| v.as_str())
          .filter(|s| !s.is_empty())
          .map(|s| s.to_string())
      })
  }

  async fn fetch_recommendations(&self, query: &str) -> LoadResult {
    let mut seed = query.to_string();

    if let Some(caps) = self.mix_regex.captures(query) {
      let mut seed_type = caps.get(1).unwrap().as_str().to_string();
      seed = caps.get(2).unwrap().as_str().to_string();

      if seed_type == "isrc" {
        if let LoadResult::Search(tracks) = self.search_internal(&format!("isrc:{}", seed)).await {
          if let Some(track) = tracks.first() {
            seed = track.info.identifier.clone();
            seed_type = "track".to_string();
          } else {
            return LoadResult::Empty {};
          }
        } else {
          return LoadResult::Empty {};
        }
      }

      let token = match self.token_tracker.get_token().await {
        Some(t) => t,
        None => return LoadResult::Empty {},
      };

      let url = format!(
        "https://spclient.wg.spotify.com/inspiredby-mix/v2/seed_to_playlist/spotify:{}:{}?response-format=json",
        seed_type, seed
      );

      let resp = self
        .client
        .get(&url)
        .bearer_auth(token)
        .header("App-Platform", "WebPlayer")
        .header("Spotify-App-Version", "1.2.81.104.g225ec0e6")
        .send()
        .await
        .ok();

      if let Some(resp) = resp {
        if resp.status().is_success() {
          if let Ok(json) = resp.json::<Value>().await {
            if let Some(playlist_uri) = json.pointer("/mediaItems/0/uri").and_then(|v| v.as_str()) {
              if let Some(id) = playlist_uri.split(':').last() {
                let mut res = self.fetch_playlist(id).await;
                if let LoadResult::Playlist(ref mut data) = res {
                  data.tracks.truncate(self.recommendations_limit);
                }
                return res;
              }
            }
          }
        }
      }
    }

    let track_id = if seed.starts_with("track:") {
      &seed["track:".len()..]
    } else {
      &seed
    };
    self.fetch_pathfinder_recommendations(track_id).await
  }

  async fn fetch_pathfinder_recommendations(&self, id: &str) -> LoadResult {
    let variables = json!({
        "uri": format!("spotify:track:{}", id),
        "limit": self.recommendations_limit
    });
    let hash = "c77098ee9d6ee8ad3eb844938722db60570d040b49f41f5ec6e7be9160a7c86b";

    let data = match self
      .partner_api_request("internalLinkRecommenderTrack", variables, hash)
      .await
    {
      Some(d) => d,
      None => return LoadResult::Empty {},
    };

    let items = data
      .pointer("/data/internalLinkRecommenderTrack/relatedTracks/items")
      .or_else(|| data.pointer("/data/seoRecommendedTrack/items"))
      .and_then(|i| i.as_array())
      .cloned()
      .unwrap_or_default();

    if items.is_empty() {
      return LoadResult::Empty {};
    }

    let mut tracks = Vec::new();
    let futs: Vec<_> = items
      .into_iter()
      .map(|item| async move { self.parse_generic_track(&item, None).await })
      .collect();

    let results = join_all(futs).await;
    for res in results {
      if let Some(track_info) = res {
        tracks.push(Track::new(track_info));
      }
    }

    if tracks.is_empty() {
      return LoadResult::Empty {};
    }

    tracks.truncate(self.recommendations_limit);

    LoadResult::Playlist(PlaylistData {
      info: PlaylistInfo {
        name: "Spotify Recommendations".to_string(),
        selected_track: 0,
      },
      plugin_info: json!({
        "type": "recommendations",
        "totalTracks": tracks.len()
      }),
      tracks,
    })
  }

  async fn search_internal(&self, query: &str) -> LoadResult {
    let variables = json!({
        "searchTerm": query,
        "offset": 0,
        "limit": self.search_limit,
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
        if let Some(track_data) = item.get("item").or_else(|| item.get("itemV2")).and_then(|v| v.get("data")) {
          // For search we allow the ISRC metadata fallback (small result set)
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

  async fn fetch_track(&self, id: &str) -> Option<TrackInfo> {
    let variables = json!({
        "uri": format!("spotify:track:{}", id)
    });

    let hash = "612585ae06ba435ad26369870deaae23b5c8800a256cd8a57e08eddc25a37294";

    let data = self
      .partner_api_request("getTrack", variables, hash)
      .await?;
    let track = data.pointer("/data/trackUnion")?;
    // Single track — ISRC metadata fallback is fine
    self.parse_generic_track(track, None).await
  }

  async fn fetch_album(&self, id: &str) -> LoadResult {
    const HASH: &str = "b9bfabef66ed756e5e13f68a942deb60bd4125ec1f1be8cc42769dc0259b4b10";
    const PAGE_LIMIT: u64 = 50;

    let base_vars = json!({
        "uri": format!("spotify:album:{}", id),
        "locale": "en",
        "offset": 0,
        "limit": PAGE_LIMIT
    });

    let data = match self
      .partner_api_request("getAlbum", base_vars.clone(), HASH)
      .await
    {
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

    let total_count = album
      .pointer("/tracksV2/totalCount")
      .and_then(|v| v.as_u64())
      .unwrap_or(0);

    let album_artwork = album
      .pointer("/coverArt/sources")
      .and_then(|s| s.as_array())
      .and_then(|s| s.first())
      .and_then(|i| i.get("url"))
      .and_then(|v| v.as_str())
      .map(|s| s.to_string());

    // Collect first page items
    let mut all_items: Vec<Value> = album
      .pointer("/tracksV2/items")
      .and_then(|i| i.as_array())
      .cloned()
      .unwrap_or_default();

    // Fetch remaining pages concurrently if needed

    if total_count > PAGE_LIMIT {
      let max_tracks = if self.album_load_limit == 0 {
        u64::MAX
      } else {
        self.album_load_limit as u64 * PAGE_LIMIT
      };
      let effective_total = total_count.min(max_tracks);

      if effective_total > PAGE_LIMIT {
        let extra = self
          .fetch_paginated_items(
            "getAlbum",
            HASH,
            base_vars,
            "/data/albumUnion/tracksV2/items",
            effective_total,
            PAGE_LIMIT,
            self.album_page_load_concurrency,
          )
          .await;
        all_items.extend(extra);
      }
    }

    let semaphore = Arc::new(Semaphore::new(self.track_resolve_concurrency));
    let futs: Vec<_> = all_items
      .into_iter()
      .take(if self.album_load_limit > 0 {
        // If limit is set, only take what we need
        (PAGE_LIMIT * self.album_load_limit as u64) as usize
      } else {
        usize::MAX
      })
      .filter_map(|item| {
        let track_data = item.get("track")?.clone();
        let semaphore = semaphore.clone();
        let artwork = album_artwork.clone();

        Some(async move {
          let _permit = semaphore.acquire().await.unwrap();
          self.parse_generic_track(&track_data, artwork).await
        })
      })
      .collect();

    let results = join_all(futs).await;
    let tracks: Vec<Track> = results.into_iter().flatten().map(Track::new).collect();

    if tracks.is_empty() {
      LoadResult::Empty {}
    } else {
      LoadResult::Playlist(PlaylistData {
        info: PlaylistInfo {
          name,
          selected_track: -1,
        },
        plugin_info: json!({ "type": "album", "url": format!("https://open.spotify.com/album/{}", id), "artworkUrl": album_artwork, "author": album.pointer("/artists/items/0/profile/name").and_then(|v| v.as_str()), "totalTracks": total_count }),
        tracks,
      })
    }
  }

  async fn fetch_playlist(&self, id: &str) -> LoadResult {
    const HASH: &str = "bb67e0af06e8d6f52b531f97468ee4acd44cd0f82b988e15c2ea47b1148efc77";
    const PAGE_LIMIT: u64 = 100;

    let base_vars = json!({
        "uri": format!("spotify:playlist:{}", id),
        "offset": 0,
        "limit": PAGE_LIMIT,
        "enableWatchFeedEntrypoint": false
    });

    let data = match self
      .partner_api_request("fetchPlaylist", base_vars.clone(), HASH)
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

    let mut all_items: Vec<Value> = playlist
      .pointer("/content/items")
      .and_then(|i| i.as_array())
      .cloned()
      .unwrap_or_default();

    let total_count = playlist
      .pointer("/content/totalCount")
      .and_then(|v| v.as_u64())
      .unwrap_or(0);

    if total_count > PAGE_LIMIT {
      let max_tracks = if self.playlist_load_limit == 0 {
        u64::MAX
      } else {
        self.playlist_load_limit as u64 * PAGE_LIMIT
      };
      let effective_total = total_count.min(max_tracks);

      if effective_total > PAGE_LIMIT {
        let extra = self
          .fetch_paginated_items(
            "fetchPlaylist",
            HASH,
            base_vars,
            "/data/playlistV2/content/items",
            effective_total,
            PAGE_LIMIT,
            self.playlist_page_load_concurrency,
          )
          .await;
        all_items.extend(extra);
      }
    }

    let semaphore = Arc::new(Semaphore::new(self.track_resolve_concurrency));
    let futs: Vec<_> = all_items
      .into_iter()
      .take(if self.playlist_load_limit > 0 {
        (PAGE_LIMIT * self.playlist_load_limit as u64) as usize
      } else {
        usize::MAX
      })
      .filter_map(|item| {
        let track_data = item
          .pointer("/item/data")
          .or_else(|| item.pointer("/itemV2/data"))?
          .clone();
        let semaphore = semaphore.clone();
        Some(async move {
          let _permit = semaphore.acquire().await.unwrap();
          self.parse_generic_track(&track_data, None).await
        })
      })
      .collect();

    let results = join_all(futs).await;
    let tracks: Vec<Track> = results.into_iter().flatten().map(Track::new).collect();

    if tracks.is_empty() {
      LoadResult::Empty {}
    } else {
      LoadResult::Playlist(PlaylistData {
        info: PlaylistInfo {
          name: name.clone(),
          selected_track: -1,
        },
        plugin_info: json!({
          "type": "playlist",
          "url": format!("https://open.spotify.com/playlist/{}", id),
          "artworkUrl": playlist.pointer("/images/items/0/sources/0/url").and_then(|v| v.as_str()),
          "author": playlist
            .get("ownerV2")
            .and_then(|v| v.get("name"))
            .and_then(|v| v.as_str())
            .or_else(|| (id.starts_with("37i9dQZ")).then_some("Spotify")),
          "totalTracks": total_count
        }),
        tracks,
      })
    }
  }

  async fn fetch_artist(&self, id: &str) -> LoadResult {
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

    let name = artist
      .get("profile")
      .and_then(|p| p.get("name"))
      .and_then(|v| v.as_str())
      .unwrap_or("Unknown Artist")
      .to_string();

    let mut tracks = Vec::new();

    if let Some(items) = artist
      .pointer("/discography/topTracks/items")
      .and_then(|i| i.as_array())
    {
      for item in items {
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
          name: name.clone(),
          selected_track: -1,
        },
        plugin_info: json!({
          "type": "artist",
          "url": format!("https://open.spotify.com/artist/{}", id),
          "artworkUrl": artist.pointer("/visuals/avatar/sources/0/url").and_then(|v| v.as_str()),
          "author": name,
          "totalTracks": tracks.len()
        }),
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
    self
      .search_prefixes
      .iter()
      .any(|p| identifier.starts_with(p))
      || self.rec_prefixes.iter().any(|p| identifier.starts_with(p))
      || self.url_regex.is_match(identifier)
  }

  fn search_prefixes(&self) -> Vec<&str> {
    self.search_prefixes.iter().map(|s| s.as_str()).collect()
  }

  async fn load(
    &self,
    identifier: &str,
    _routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
  ) -> LoadResult {
    if let Some(prefix) = self
      .search_prefixes
      .iter()
      .find(|p| identifier.starts_with(*p))
    {
      let query = &identifier[prefix.len()..];
      return self.search_internal(query).await;
    }

    if let Some(prefix) = self
      .rec_prefixes
      .iter()
      .find(|p| identifier.starts_with(*p))
    {
      let query = &identifier[prefix.len()..];
      return self.fetch_recommendations(query).await;
    }

    if let Some(caps) = self.url_regex.captures(identifier) {
      let type_str = match caps.get(1) {
        Some(m) => m.as_str(),
        None => return LoadResult::Empty {},
      };
      let id = match caps.get(2) {
        Some(m) => m.as_str(),
        None => return LoadResult::Empty {},
      };

      match type_str {
        "track" => {
          if let Some(track_info) = self.fetch_track(id).await {
            return LoadResult::Track(Track::new(track_info));
          }
        }
        "album" => return self.fetch_album(id).await,
        "playlist" => return self.fetch_playlist(id).await,
        "artist" => return self.fetch_artist(id).await,
        _ => {}
      }
    }

    if let Some(caps) = self.track_regex.captures(identifier) {
      let id = match caps.get(1) {
        Some(m) => m.as_str(),
        None => return LoadResult::Empty {},
      };
      if let Some(track_info) = self.fetch_track(id).await {
        return LoadResult::Track(Track::new(track_info));
      }
    }

    if let Some(caps) = self.album_regex.captures(identifier) {
      let id = match caps.get(1) {
        Some(m) => m.as_str(),
        None => return LoadResult::Empty {},
      };
      return self.fetch_album(id).await;
    }

    if let Some(caps) = self.playlist_regex.captures(identifier) {
      let id = match caps.get(1) {
        Some(m) => m.as_str(),
        None => return LoadResult::Empty {},
      };
      return self.fetch_playlist(id).await;
    }

    if let Some(caps) = self.artist_regex.captures(identifier) {
      let id = match caps.get(1) {
        Some(m) => m.as_str(),
        None => return LoadResult::Empty {},
      };
      return self.fetch_artist(id).await;
    }

    LoadResult::Empty {}
  }
}
