use std::sync::Arc;

use async_trait::async_trait;
use regex::Regex;
use reqwest::header::{HeaderMap, HeaderValue, USER_AGENT};
use serde_json::Value;
use tracing::{error, warn};

use super::token::TidalTokenTracker;
use crate::{
  api::tracks::{LoadResult, PlaylistData, PlaylistInfo, Track, TrackInfo},
  sources::SourcePlugin,
};

const API_BASE: &str = "https://api.tidal.com/v1";

pub struct TidalSource {
  client: reqwest::Client,
  token_tracker: Arc<TidalTokenTracker>,
  country_code: String,

  #[allow(dead_code)]
  playlist_load_limit: usize,
  #[allow(dead_code)]
  album_load_limit: usize,
  #[allow(dead_code)]
  artist_load_limit: usize,

  search_prefixes: Vec<String>,
  rec_prefixes: Vec<String>,
  url_regex: Regex,
}

impl TidalSource {
  pub fn new(config: Option<crate::configs::TidalConfig>) -> Result<Self, String> {
    let mut headers = HeaderMap::new();
    headers.insert(
      USER_AGENT,
      HeaderValue::from_static("TIDAL/3704 CFNetwork/1220.1 Darwin/20.3.0"),
    );
    headers.insert("Accept-Language", HeaderValue::from_static("en-US"));

    let client = reqwest::Client::builder()
      .default_headers(headers)
      .build()
      .map_err(|e| e.to_string())?;

    let (country, p_limit, a_limit, art_limit, token) = if let Some(c) = config {
      (
        c.country_code,
        c.playlist_load_limit,
        c.album_load_limit,
        c.artist_load_limit,
        c.token,
      )
    } else {
      ("US".to_string(), 0, 0, 0, None)
    };

    let token_tracker = Arc::new(TidalTokenTracker::new(client.clone(), token));
    token_tracker.clone().init();

    Ok(Self {
      token_tracker,
      client,
      country_code: country,
      playlist_load_limit: p_limit,
      album_load_limit: a_limit,
      artist_load_limit: art_limit,
      search_prefixes: vec!["tdsearch:".to_string()],
      rec_prefixes: vec!["tdrec:".to_string()],
      url_regex: Regex::new(r"https?://(?:(?:listen|www)\.)?tidal\.com/(?:browse/)?(album|track|playlist|mix|artist)/([a-zA-Z0-9\-]+)(?:/.*)?(?:\?.*)?").unwrap(),
    })
  }

  async fn api_request(&self, path: &str) -> Option<Value> {
    let token = self.token_tracker.get_token().await?;

    let url = if path.starts_with("http") {
      path.to_string()
    } else {
      format!("{}{}", API_BASE, path)
    };

    // Append country code
    let url = if url.contains('?') {
      format!("{}&countryCode={}", url, self.country_code)
    } else {
      format!("{}?countryCode={}", url, self.country_code)
    };

    let req = self.client.get(&url).header("x-tidal-token", token);

    let resp = match req.send().await {
      Ok(r) => r,
      Err(e) => {
        error!("Tidal API request failed: {}", e);
        return None;
      }
    };

    if !resp.status().is_success() {
      warn!("Tidal API returned {}", resp.status());
      return None;
    }

    resp.json().await.ok()
  }

  fn parse_track(&self, item: &Value) -> Option<TrackInfo> {
    let id = item.get("id")?.as_u64()?.to_string();
    let title = item
      .get("title")
      .and_then(|v| v.as_str())
      .unwrap_or("Unknown Title")
      .to_string();

    let artists = item
      .get("artists")
      .and_then(|v| v.as_array())
      .map(|a| {
        a.iter()
          .filter_map(|x| x.get("name").and_then(|n| n.as_str()))
          .collect::<Vec<_>>()
          .join(", ")
      })
      .unwrap_or_else(|| "Unknown Artist".to_string());

    let length = item.get("duration").and_then(|v| v.as_u64()).unwrap_or(0) * 1000;

    let isrc = item
      .get("isrc")
      .and_then(|v| v.as_str())
      .filter(|s| !s.is_empty())
      .map(|s| s.to_string());

    let artwork_url = item
      .get("album")
      .and_then(|a| a.get("cover"))
      .and_then(|v| {
        v.as_str().filter(|s| !s.is_empty()).map(|s| {
          format!(
            "https://resources.tidal.com/images/{}/1280x1280.jpg",
            s.replace("-", "/")
          )
        })
      });

    let url = item
      .get("url")
      .and_then(|v| v.as_str())
      .filter(|s| !s.is_empty())
      .map(|s| s.replace("http://", "https://"));

    Some(TrackInfo {
      title,
      author: artists,
      length,
      identifier: id,
      is_stream: false,
      uri: url,
      artwork_url,
      isrc,
      source_name: "tidal".to_string(),
      is_seekable: true,
      position: 0,
    })
  }

  async fn get_track_data(&self, id: &str) -> LoadResult {
    let path = format!("/tracks/{}", id);
    let data = match self.api_request(&path).await {
      Some(d) => d,
      None => return LoadResult::Empty {},
    };

    if let Some(info) = self.parse_track(&data) {
      return LoadResult::Track(Track::new(info));
    }
    LoadResult::Empty {}
  }

  async fn get_album_or_playlist(&self, id: &str, type_str: &str) -> LoadResult {
    // First get album/playlist info for metadata
    let info_path = format!("/{}s/{}", type_str, id);
    let info_data = match self.api_request(&info_path).await {
      Some(d) => d,
      None => return LoadResult::Empty {},
    };

    let title = info_data
      .get("title")
      .and_then(|v| v.as_str())
      .unwrap_or("Unknown")
      .to_string();

    // Fetch tracks
    let tracks_path = format!("/{}s/{}/tracks?limit=100", type_str, id); // Simplified limit for now
    let tracks_data = match self.api_request(&tracks_path).await {
      Some(d) => d,
      None => return LoadResult::Empty {},
    };

    let items = tracks_data.get("items").and_then(|v| v.as_array());

    let mut tracks = Vec::new();
    if let Some(list) = items {
      for item in list {
        // Playlist items wrap the track in an "item" object, albums don't?
        // NodeLink says: this._parseTrack(item.item || item)
        let track_obj = if item.get("item").is_some() {
          item.get("item").unwrap()
        } else {
          item
        };

        if let Some(info) = self.parse_track(track_obj) {
          tracks.push(Track::new(info));
        }
      }
    }

    if tracks.is_empty() {
      return LoadResult::Empty {};
    }

    LoadResult::Playlist(PlaylistData {
      info: PlaylistInfo {
        name: title,
        selected_track: -1,
      },
      plugin_info: serde_json::json!({ "type": type_str, "url": format!("https://tidal.com/browse/{}/{}", type_str, id), "artworkUrl": info_data.get("cover").or_else(|| info_data.get("image")).and_then(|v| v.as_str()).map(|s| format!("https://resources.tidal.com/images/{}/1280x1280.jpg", s.replace("-", "/"))), "author": info_data.get("artist").and_then(|a| a.get("name")).or_else(|| info_data.get("creator").and_then(|c| c.get("name"))).and_then(|v| v.as_str()), "totalTracks": info_data.get("numberOfTracks").or_else(|| info_data.get("numberOfSongs")).and_then(|v| v.as_u64()).unwrap_or(tracks.len() as u64) }),
      tracks,
    })
  }

  async fn get_mix(&self, id: &str, name_override: Option<String>) -> LoadResult {
    let path = format!("/mixes/{}/items?limit=100", id);
    let data = match self.api_request(&path).await {
      Some(d) => d,
      None => return LoadResult::Empty {},
    };

    let items = data.get("items").and_then(|v| v.as_array());

    let mut tracks = Vec::new();
    if let Some(list) = items {
      for item in list {
        let track_obj = if item.get("item").is_some() {
          item.get("item").unwrap()
        } else {
          item
        };

        if let Some(info) = self.parse_track(track_obj) {
          tracks.push(Track::new(info));
        }
      }
    }

    if tracks.is_empty() {
      return LoadResult::Empty {};
    }

    let name = name_override.unwrap_or_else(|| format!("Mix: {}", id));
    LoadResult::Playlist(PlaylistData {
      info: PlaylistInfo {
        name,
        selected_track: -1,
      },
      plugin_info: serde_json::json!({ "type": "playlist", "url": format!("https://tidal.com/browse/mix/{}", id), "totalTracks": tracks.len() }),
      tracks,
    })
  }

  async fn search(&self, query: &str) -> LoadResult {
    let encoded_query = urlencoding::encode(query);
    let path = format!("/search?query={}&limit=10&types=TRACKS", encoded_query);

    let data = match self.api_request(&path).await {
      Some(d) => d,
      None => return LoadResult::Empty {},
    };

    let items = data.pointer("/tracks/items").and_then(|v| v.as_array());

    let mut tracks = Vec::new();
    if let Some(list) = items {
      for item in list {
        if let Some(info) = self.parse_track(item) {
          tracks.push(Track::new(info));
        }
      }
    }

    if tracks.is_empty() {
      LoadResult::Empty {}
    } else {
      LoadResult::Search(tracks)
    }
  }

  async fn get_recommendations(&self, id: &str) -> LoadResult {
    let path = format!("/tracks/{}", id);
    let data = match self.api_request(&path).await {
      Some(d) => d,
      None => return LoadResult::Empty {},
    };

    if let Some(mix_id) = data.pointer("/mixes/TRACK_MIX").and_then(|v| v.as_str()) {
      return self
        .get_mix(mix_id, Some("Tidal Recommendations".to_string()))
        .await;
    }

    LoadResult::Empty {}
  }
  async fn get_artist_top_tracks(&self, id: &str) -> LoadResult {
    // First get artist info for name
    let info_path = format!("/artists/{}", id);
    let info_data = match self.api_request(&info_path).await {
      Some(d) => d,
      None => return LoadResult::Empty {},
    };

    let artist_name = info_data
      .get("name")
      .and_then(|v| v.as_str())
      .unwrap_or("Unknown Artist")
      .to_string();

    let path = format!("/artists/{}/toptracks?limit=10", id);
    let data = match self.api_request(&path).await {
      Some(d) => d,
      None => return LoadResult::Empty {},
    };

    let items = data.get("items").and_then(|v| v.as_array());

    let mut tracks = Vec::new();
    if let Some(list) = items {
      for item in list {
        if let Some(info) = self.parse_track(item) {
          tracks.push(Track::new(info));
        }
      }
    }

    // Apply limit if configured (though API limit is 10 usually)
    if self.artist_load_limit > 0 && tracks.len() > self.artist_load_limit {
      tracks.truncate(self.artist_load_limit);
    }

    if tracks.is_empty() {
      return LoadResult::Empty {};
    }

    LoadResult::Playlist(PlaylistData {
      info: PlaylistInfo {
        name: format!("{}'s Top Tracks", artist_name),
        selected_track: -1,
      },
      plugin_info: serde_json::json!({ "type": "artist", "url": format!("https://tidal.com/browse/artist/{}", id), "artworkUrl": info_data.get("picture").and_then(|v| v.as_str()).map(|s| format!("https://resources.tidal.com/images/{}/1280x1280.jpg", s.replace("-", "/"))), "author": artist_name, "totalTracks": tracks.len() }),
      tracks,
    })
  }
}

#[async_trait]
impl SourcePlugin for TidalSource {
  fn name(&self) -> &str {
    "tidal"
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
      return self.search(query).await;
    }

    if let Some(prefix) = self
      .rec_prefixes
      .iter()
      .find(|p| identifier.starts_with(*p))
    {
      let id = &identifier[prefix.len()..];
      return self.get_recommendations(id).await;
    }

    if let Some(caps) = self.url_regex.captures(identifier) {
      let type_str = caps.get(1).map(|m| m.as_str()).unwrap_or("");
      let id = caps.get(2).map(|m| m.as_str()).unwrap_or("");

      match type_str {
        "track" => return self.get_track_data(id).await,
        "album" => return self.get_album_or_playlist(id, "album").await,
        "playlist" => return self.get_album_or_playlist(id, "playlist").await,
        "mix" => return self.get_mix(id, None).await,
        "artist" => return self.get_artist_top_tracks(id).await,
        _ => return LoadResult::Empty {},
      }
    }

    LoadResult::Empty {}
  }
}
