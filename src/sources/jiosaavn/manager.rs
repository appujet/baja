use std::sync::Arc;

use async_trait::async_trait;
use regex::Regex;
use reqwest::header::HeaderMap;
use serde_json::Value;
use tracing::{debug, warn};

use super::track::JioSaavnTrack;
use crate::{
  api::tracks::{LoadError, LoadResult, PlaylistData, PlaylistInfo, Track, TrackInfo},
  sources::plugin::{PlayableTrack, SourcePlugin},
};

const API_BASE: &str = "https://www.jiosaavn.com/api.php";

pub struct JioSaavnSource {
  client: reqwest::Client,
  url_regex: Regex,
  search_prefixes: Vec<String>,
  rec_prefixes: Vec<String>,
  secret_key: Vec<u8>,
  proxy: Option<crate::configs::HttpProxyConfig>,
  // Limits
  search_limit: usize,
  recommendations_limit: usize,
  playlist_load_limit: usize,
  album_load_limit: usize,
  artist_load_limit: usize,
}

impl JioSaavnSource {
  pub fn new(config: Option<crate::configs::JioSaavnConfig>) -> Result<Self, String> {
    let mut headers = HeaderMap::new();

    headers.insert(
      "User-Agent",
      "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/115.0.0.0 Safari/537.36"
        .parse()
        .unwrap(),
    );

    headers.insert("Accept", "application/json".parse().unwrap());
    headers.insert("Accept-Language", "en-US,en;q=0.9".parse().unwrap());
    headers.insert("Referer", "https://www.jiosaavn.com/".parse().unwrap());
    headers.insert("Origin", "https://www.jiosaavn.com".parse().unwrap());

    let (
      secret_key,
      search_limit,
      recommendations_limit,
      playlist_load_limit,
      album_load_limit,
      artist_load_limit,
      proxy,
    ) = if let Some(c) = config {
      (
        c.decryption
          .and_then(|d| d.secret_key)
          .unwrap_or_else(|| "38346591".to_string()),
        c.search_limit,
        c.recommendations_limit,
        c.playlist_load_limit,
        c.album_load_limit,
        c.artist_load_limit,
        c.proxy,
      )
    } else {
      ("38346591".to_string(), 10, 10, 50, 50, 20, None)
    };

    let mut client_builder = reqwest::Client::builder().default_headers(headers);

    if let Some(proxy_config) = &proxy {
      if let Some(url) = &proxy_config.url {
        debug!("Configuring proxy for JioSaavnSource: {}", url);
        if let Ok(proxy_obj) = reqwest::Proxy::all(url) {
          let mut proxy_obj = proxy_obj;
          if let (Some(username), Some(password)) = (&proxy_config.username, &proxy_config.password)
          {
            proxy_obj = proxy_obj.basic_auth(username, password);
          }
          client_builder = client_builder.proxy(proxy_obj);
        }
      }
    }

    let client = client_builder.build().map_err(|e| e.to_string())?;

    Ok(Self {
      client,
      url_regex: Regex::new(r"https?://(?:www\.)?jiosaavn\.com/(?:(?<type>album|featured|song|s/playlist|artist)/)(?:[^/]+/)(?<id>[A-Za-z0-9_,-]+)").unwrap(),
      search_prefixes: vec!["jssearch:".to_string()],
      rec_prefixes: vec!["jsrec:".to_string()],
      secret_key: secret_key.into_bytes(),
      proxy,
      search_limit,
      recommendations_limit,
      playlist_load_limit,
      album_load_limit,
      artist_load_limit,
    })
  }

  async fn get_json(&self, params: &[(&str, &str)]) -> Option<Value> {
    let resp = match self.client.get(API_BASE).query(params).send().await {
      Ok(r) => r,
      Err(e) => {
        warn!("JioSaavn request failed: {}", e);
        return None;
      }
    };

    if !resp.status().is_success() {
      warn!("JioSaavn API error status: {}", resp.status());
      return None;
    }

    let text = match resp.text().await {
      Ok(text) => text,
      Err(e) => {
        warn!("Failed to read response body: {}", e);
        return None;
      }
    };
    serde_json::from_str(&text).ok()
  }

  fn clean_string(&self, s: &str) -> String {
    s.replace("&quot;", "\"").replace("&amp;", "&")
  }

  fn parse_track(&self, json: &Value) -> Option<Track> {
    let id = json.get("id").and_then(|v| {
      v.as_str()
        .map(|s| s.to_string())
        .or_else(|| v.as_i64().map(|i| i.to_string()))
    })?;

    let title_raw = json.get("title").or_else(|| json.get("song"))?.as_str()?;
    let title = self.clean_string(title_raw);

    let uri = json
      .get("perma_url")
      .and_then(|v| v.as_str())
      .filter(|s| !s.is_empty())
      .map(|s| s.to_string());

    let duration_str = json
      .get("more_info")
      .and_then(|m| m.get("duration"))
      .or_else(|| json.get("duration"))
      .and_then(|v| v.as_str())
      .unwrap_or("0");
    let duration = duration_str.parse::<u64>().unwrap_or(0) * 1000;

    let primary_artists = json
      .get("more_info")
      .and_then(|m| m.get("artistMap"))
      .and_then(|am| am.get("primary_artists"));
    let artists = json
      .get("more_info")
      .and_then(|m| m.get("artistMap"))
      .and_then(|am| am.get("artists"));

    let meta_artists = if let Some(arr) = primary_artists.and_then(|v| v.as_array()) {
      if !arr.is_empty() {
        Some(arr)
      } else {
        artists.and_then(|v| v.as_array())
      }
    } else {
      artists.and_then(|v| v.as_array())
    };

    let author = if let Some(arr) = meta_artists {
      arr
        .iter()
        .filter_map(|a| a.get("name").and_then(|v| v.as_str()))
        .collect::<Vec<_>>()
        .join(", ")
    } else {
      json
        .get("more_info")
        .and_then(|m| m.get("music"))
        .or_else(|| json.get("primary_artists"))
        .or_else(|| json.get("singers"))
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown Artist")
        .to_string()
    };
    let author = self.clean_string(&author);

    let artwork_url = json
      .get("image")
      .and_then(|v| v.as_str())
      .filter(|s| !s.is_empty())
      .map(|s| s.replace("150x150", "500x500"));

    let track_info = TrackInfo {
      identifier: id,
      is_seekable: true,
      author,
      length: duration,
      is_stream: false,
      position: 0,
      title,
      uri,
      artwork_url,
      isrc: None,
      source_name: "jiosaavn".to_string(),
    };

    Some(Track::new(track_info))
  }

  async fn fetch_metadata(&self, id: &str) -> Option<Value> {
    let params = vec![
      ("__call", "webapi.get"),
      ("api_version", "4"),
      ("_format", "json"),
      ("_marker", "0"),
      ("ctx", "web6dot0"),
      ("token", id),
      ("type", "song"),
    ];

    self.get_json(&params).await.and_then(|json| {
      json
        .get("songs")
        .and_then(|s| s.get(0))
        .cloned()
        .or_else(|| {
          if json.get("id").is_some() {
            Some(json)
          } else {
            None
          }
        })
    })
  }

  async fn search(&self, query: &str) -> LoadResult {
    debug!("JioSaavn searching: {}", query);

    let params = vec![
      ("__call", "search.getResults"),
      ("api_version", "4"),
      ("_format", "json"),
      ("_marker", "0"),
      ("cc", "in"),
      ("ctx", "web6dot0"),
      ("includeMetaTags", "1"),
      ("q", query),
    ];

    if let Some(json) = self.get_json(&params).await {
      if let Some(results) = json.get("results").and_then(|v| v.as_array()) {
        if results.is_empty() {
          return LoadResult::Empty {};
        }
        let tracks: Vec<Track> = results
          .iter()
          .take(self.search_limit)
          .filter_map(|item| self.parse_track(item))
          .collect();
        return LoadResult::Search(tracks);
      }
      LoadResult::Empty {}
    } else {
      LoadResult::Error(LoadError {
        message: "JioSaavn search failed".to_string(),
        severity: crate::common::Severity::Common,
        cause: "".to_string(),
      })
    }
  }

  async fn get_recommendations(&self, query: &str) -> LoadResult {
    let mut id = query.to_string();
    let id_regex = Regex::new(r"^[A-Za-z0-9_,-]+$").unwrap();
    if !id_regex.is_match(query) {
      if let LoadResult::Search(tracks) = self.search(query).await {
        if let Some(first) = tracks.first() {
          id = first.info.identifier.clone();
        } else {
          return LoadResult::Empty {};
        }
      } else {
        return LoadResult::Empty {};
      }
    }

    let encoded_id = format!("[\"{}\"]", id);

    let params = vec![
      ("__call", "webradio.createEntityStation"),
      ("api_version", "4"),
      ("_format", "json"),
      ("_marker", "0"),
      ("ctx", "android"),
      ("entity_id", &encoded_id),
      ("entity_type", "queue"),
    ];

    let station_id = self.get_json(&params).await.and_then(|json| {
      json
        .get("stationid")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
    });

    if let Some(sid) = station_id {
      let k_limit = self.recommendations_limit.to_string();
      let params = vec![
        ("__call", "webradio.getSong"),
        ("api_version", "4"),
        ("_format", "json"),
        ("_marker", "0"),
        ("ctx", "android"),
        ("stationid", &sid),
        ("k", &k_limit),
      ];

      if let Some(json) = self.get_json(&params).await {
        if let Some(obj) = json.as_object() {
          let tracks: Vec<Track> = obj
            .values()
            .filter_map(|v| v.get("song"))
            .filter_map(|song| self.parse_track(song))
            .collect();

          if !tracks.is_empty() {
            return LoadResult::Playlist(PlaylistData {
              info: PlaylistInfo {
                name: "JioSaavn Recommendations".to_string(),
                selected_track: 0,
              },
              plugin_info: serde_json::json!({
                "type": "recommendations",
                "totalTracks": tracks.len()
              }),
              tracks,
            });
          }
        }
      }
    }

    if let Some(metadata) = self.fetch_metadata(&id).await {
      if let Some(artist_ids) = metadata.get("primary_artists_id").and_then(|v| v.as_str()) {
        let params = vec![
          ("__call", "search.artistOtherTopSongs"),
          ("api_version", "4"),
          ("_format", "json"),
          ("_marker", "0"),
          ("ctx", "wap6dot0"),
          ("artist_ids", artist_ids),
          ("song_id", &id),
          ("language", "unknown"),
        ];

        if let Some(json) = self.get_json(&params).await {
          if let Some(arr) = json.as_array() {
            let tracks: Vec<Track> = arr
              .iter()
              .take(self.recommendations_limit)
              .filter_map(|item| self.parse_track(item))
              .collect();

            if !tracks.is_empty() {
              return LoadResult::Playlist(PlaylistData {
                info: PlaylistInfo {
                  name: "JioSaavn Recommendations".to_string(),
                  selected_track: -1,
                },
                plugin_info: serde_json::json!({
                  "type": "recommendations",
                  "totalTracks": tracks.len()
                }),
                tracks,
              });
            }
          }
        }
      }
    }

    LoadResult::Empty {}
  }

  async fn resolve_list(&self, type_: &str, id: &str) -> LoadResult {
    let type_param = if type_ == "featured" || type_ == "s/playlist" {
      "playlist"
    } else {
      type_
    };

    let n_songs = if type_ == "artist" {
      self.artist_load_limit
    } else if type_ == "album" {
      self.album_load_limit
    } else {
      self.playlist_load_limit
    };
    let n_songs_str = n_songs.to_string();

    let mut params = vec![
      ("__call", "webapi.get"),
      ("api_version", "4"),
      ("_format", "json"),
      ("_marker", "0"),
      ("ctx", "web6dot0"),
      ("token", id),
      ("type", type_param),
    ];

    if type_ == "artist" {
      params.push(("n_song", &n_songs_str));
    } else {
      params.push(("n", &n_songs_str));
    }

    if let Some(data) = self.get_json(&params).await {
      let list = data.get("list").or_else(|| data.get("topSongs"));
      if let Some(arr) = list.and_then(|v| v.as_array()) {
        if arr.is_empty() {
          return LoadResult::Empty {};
        }

        let tracks: Vec<Track> = arr
          .iter()
          .filter_map(|item| self.parse_track(item))
          .collect();

        let name_raw = data
          .get("title")
          .or_else(|| data.get("name"))
          .and_then(|v| v.as_str())
          .unwrap_or("");
        let mut name = self.clean_string(name_raw);

        if type_ == "artist" {
          name = format!("{}'s Top Tracks", name);
        }

        LoadResult::Playlist(PlaylistData {
          info: PlaylistInfo {
            name,
            selected_track: -1,
          },
          plugin_info: serde_json::json!({
            "url": data.get("perma_url").and_then(|v| v.as_str()),
            "type": type_,
            "artworkUrl": data.get("image").and_then(|v| v.as_str()).map(|s| s.replace("150x150", "500x500")),
            "author": data.get("subtitle").or_else(|| data.get("header_desc")).and_then(|v| v.as_str()),
            "totalTracks": data.get("list_count").and_then(|v| v.as_str()).and_then(|s| s.parse::<u64>().ok()).unwrap_or(tracks.len() as u64)
          }),
          tracks,
        })
      } else {
        LoadResult::Empty {}
      }
    } else {
      LoadResult::Error(LoadError {
        message: "JioSaavn list fetch failed".to_string(),
        severity: crate::common::Severity::Common,
        cause: "".to_string(),
      })
    }
  }

  async fn get_autocomplete(
    &self,
    query: &str,
    types: &[String],
  ) -> Option<crate::api::tracks::SearchResult> {
    debug!("JioSaavn get_autocomplete: {}", query);

    let params = vec![
      ("__call", "autocomplete.get"),
      ("api_version", "4"),
      ("_format", "json"),
      ("_marker", "0"),
      ("ctx", "web6dot0"),
      ("query", query),
    ];

    let json = self.get_json(&params).await?;

    let mut tracks = Vec::new();
    let mut albums = Vec::new();
    let mut artists = Vec::new();
    let mut playlists = Vec::new();
    let mut texts = Vec::new();

    let all_types = types.is_empty();

    // Parse Songs -> Tracks
    if all_types || types.contains(&"track".to_string()) {
      if let Some(songs) = json
        .get("songs")
        .and_then(|v| v.get("data"))
        .and_then(|v| v.as_array())
      {
        for item in songs {
          if let Some(track) = self.parse_search_item(item, "song") {
            tracks.push(track);
          }
        }
      }
    }

    // Batch update track details for duration and perma_url
    if !tracks.is_empty() {
      let pids: Vec<String> = tracks.iter().map(|t| t.info.identifier.clone()).collect();
      let pids_str = pids.join(",");
      let details_params = vec![
        ("__call", "song.getDetails"),
        ("_format", "json"),
        ("pids", &pids_str),
      ];

      if let Some(details_json) = self.get_json(&details_params).await {
        for track in &mut tracks {
          if let Some(detail) = details_json.get(&track.info.identifier) {
            // Update length
            if let Some(duration) = detail
              .get("duration")
              .and_then(|v| v.as_str())
              .and_then(|s| s.parse::<u64>().ok())
              .or_else(|| detail.get("duration").and_then(|v| v.as_u64()))
            {
              track.info.length = duration * 1000;
            }

            // Update URI
            if let Some(perma_url) = detail.get("perma_url").and_then(|v| v.as_str()) {
              track.info.uri = Some(perma_url.to_string());
            }

            // Update Author
            if let Some(artists) = detail.get("primary_artists").and_then(|v| v.as_str()) {
              if !artists.is_empty() {
                track.info.author = self.clean_string(artists);
              }
            }

            // Re-encode track with updated info
            track.encoded = track.encode();
          }
        }
      }
    }

    // Parse Albums -> PlaylistData
    if all_types || types.contains(&"album".to_string()) {
      if let Some(data) = json
        .get("albums")
        .and_then(|v| v.get("data"))
        .and_then(|v| v.as_array())
      {
        for item in data {
          if let Some(pd) = self.parse_search_playlist(item, "album") {
            albums.push(pd);
          }
        }
      }
    }

    // Parse Artists -> PlaylistData
    if all_types || types.contains(&"artist".to_string()) {
      if let Some(data) = json
        .get("artists")
        .and_then(|v| v.get("data"))
        .and_then(|v| v.as_array())
      {
        for item in data {
          if let Some(pd) = self.parse_search_playlist(item, "artist") {
            artists.push(pd);
          }
        }
      }
    }

    // Parse Playlists -> PlaylistData
    if all_types || types.contains(&"playlist".to_string()) {
      if let Some(data) = json
        .get("playlists")
        .and_then(|v| v.get("data"))
        .and_then(|v| v.as_array())
      {
        for item in data {
          if let Some(pd) = self.parse_search_playlist(item, "playlist") {
            playlists.push(pd);
          }
        }
      }
    }

    // Parse TopQuery -> TextData
    if all_types || types.contains(&"text".to_string()) {
      if let Some(data) = json
        .get("topquery")
        .and_then(|v| v.get("data"))
        .and_then(|v| v.as_array())
      {
        for item in data {
          if let Some(text) = item.get("title").and_then(|v| v.as_str()) {
            texts.push(crate::api::tracks::TextData {
              text: self.clean_string(text),
              plugin_info: serde_json::json!({}),
            });
          }
        }
      }
    }

    Some(crate::api::tracks::SearchResult {
      tracks,
      albums,
      artists,
      playlists,
      texts,
      plugin_info: serde_json::json!({}),
    })
  }

  fn parse_search_item(&self, json: &Value, _type: &str) -> Option<Track> {
    let id = json.get("id")?.as_str()?;
    let title = self.clean_string(json.get("title")?.as_str()?);
    let author = self.clean_string(
      json
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown Artist"),
    );
    let uri = json
      .get("url")
      .and_then(|v| v.as_str())
      .map(|s| s.to_string());
    let artwork_url = json
      .get("image")
      .and_then(|v| v.as_str())
      .map(|s| s.replace("150x150", "500x500"));

    let track_info = TrackInfo {
      identifier: id.to_string(),
      is_seekable: true,
      author,
      length: 0,
      is_stream: false,
      position: 0,
      title,
      uri,
      artwork_url,
      isrc: None,
      source_name: "jiosaavn".to_string(),
    };

    Some(Track::new(track_info))
  }

  fn parse_search_playlist(&self, json: &Value, type_: &str) -> Option<PlaylistData> {
    let title = self.clean_string(json.get("title")?.as_str()?);
    let uri = json
      .get("url")
      .and_then(|v| v.as_str())
      .unwrap_or("")
      .to_string();

    Some(PlaylistData {
      info: PlaylistInfo {
        name: title,
        selected_track: -1,
      },
      plugin_info: serde_json::json!({
          "url": uri,
          "type": type_,
          "artworkUrl": json.get("image").and_then(|v| v.as_str()).map(|s| s.replace("150x150", "500x500")),
          "author": json.get("music").or_else(|| json.get("description")).and_then(|v| v.as_str()),
          "totalTracks": json.get("more_info").and_then(|m| m.get("song_count").or_else(|| m.get("track_count"))).and_then(|v| v.as_str().and_then(|s| s.parse::<u64>().ok()).or_else(|| v.as_u64())).unwrap_or(0)
      }),
      tracks: Vec::new(),
    })
  }
}

#[async_trait]
impl SourcePlugin for JioSaavnSource {
  fn name(&self) -> &str {
    "jiosaavn"
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

  fn rec_prefixes(&self) -> Vec<&str> {
    self.rec_prefixes.iter().map(|s| s.as_str()).collect()
  }

  async fn load(
    &self,
    identifier: &str,
    _routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
  ) -> LoadResult {
    if let Some(prefix) = self
      .rec_prefixes
      .iter()
      .find(|p| identifier.starts_with(*p))
    {
      let query = identifier.strip_prefix(prefix).unwrap();
      return self.get_recommendations(query).await;
    }

    if let Some(prefix) = self
      .search_prefixes
      .iter()
      .find(|p| identifier.starts_with(*p))
    {
      let query = identifier.strip_prefix(prefix).unwrap();
      return self.search(query).await;
    }

    // Regex Match URL
    if let Some(caps) = self.url_regex.captures(identifier) {
      let type_ = caps.name("type").map(|m| m.as_str()).unwrap_or("");
      let id = caps.name("id").map(|m| m.as_str()).unwrap_or("");

      if id.is_empty() || type_.is_empty() {
        return LoadResult::Empty {};
      }

      if type_ == "song" {
        if let Some(track_data) = self.fetch_metadata(id).await {
          if let Some(track) = self.parse_track(&track_data) {
            return LoadResult::Track(track);
          }
        }
        return LoadResult::Empty {};
      } else {
        return self.resolve_list(type_, id).await;
      }
    }

    LoadResult::Empty {}
  }

  async fn get_track(
    &self,
    identifier: &str,
    routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
  ) -> Option<Box<dyn PlayableTrack>> {
    let id = if let Some(caps) = self.url_regex.captures(identifier) {
      caps.name("id").map(|m| m.as_str()).unwrap_or(identifier)
    } else {
      identifier
    };

    let track_data = self.fetch_metadata(id).await?;
    let encrypted_url = track_data
      .get("more_info")
      .and_then(|m| m.get("encrypted_media_url"))
      .and_then(|v| v.as_str())?
      .to_string();

    let is_320 = track_data
      .get("more_info")
      .and_then(|m| m.get("320kbps"))
      .map(|v| v.as_str() == Some("true") || v.as_bool() == Some(true))
      .unwrap_or(false);

    let local_addr = if let Some(rp) = routeplanner {
      rp.get_address()
    } else {
      None
    };

    Some(Box::new(JioSaavnTrack {
      encrypted_url,
      secret_key: self.secret_key.clone(),
      is_320,
      local_addr,
      proxy: self.proxy.clone(),
    }))
  }

  fn get_proxy_config(&self) -> Option<crate::configs::HttpProxyConfig> {
    self.proxy.clone()
  }

  async fn load_search(
    &self,
    query: &str,
    types: &[String],
    _routeplanner: Option<Arc<dyn crate::routeplanner::RoutePlanner>>,
  ) -> Option<crate::api::tracks::SearchResult> {
    let q = if let Some(prefix) = self.search_prefixes.iter().find(|p| query.starts_with(*p)) {
      query.strip_prefix(prefix).unwrap()
    } else {
      query
    };

    self.get_autocomplete(q, types).await
  }
}
