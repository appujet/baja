use std::collections::HashSet;
use crate::api::tracks::{LoadResult, PlaylistData, PlaylistInfo};
use super::AppleMusicSource;
use super::API_BASE;

impl AppleMusicSource {
  pub(crate) async fn search(&self, query: &str) -> LoadResult {
    let encoded_query = urlencoding::encode(query);
    let path = format!(
      "/catalog/{}/search?term={}&limit=10&types=songs",
      self.country_code, encoded_query
    );

    let data = match self.api_request(&path).await {
      Some(d) => d,
      None => return LoadResult::Empty {},
    };

    let songs = data
      .pointer("/results/songs/data")
      .and_then(|v| v.as_array());

    let mut tracks = Vec::new();
    if let Some(items) = songs {
      for item in items {
        if let Some(track) = self.build_track(item, None) {
          tracks.push(track);
        }
      }
    }

    if tracks.is_empty() {
      LoadResult::Empty {}
    } else {
      LoadResult::Search(tracks)
    }
  }

  pub(crate) async fn get_search_suggestions(
    &self,
    query: &str,
    types: &[String],
  ) -> Option<crate::api::tracks::SearchResult> {
    let mut kinds = HashSet::new();
    let mut am_types = Vec::new();

    let all_types = types.is_empty();

    if all_types
      || types.contains(&"track".to_string())
      || types.contains(&"album".to_string())
      || types.contains(&"artist".to_string())
      || types.contains(&"playlist".to_string())
    {
      kinds.insert("topResults");
    }

    if types.contains(&"text".to_string()) {
      kinds.insert("terms");
    }

    if all_types || types.contains(&"track".to_string()) {
      am_types.push("songs");
    }
    if all_types || types.contains(&"album".to_string()) {
      am_types.push("albums");
    }
    if all_types || types.contains(&"artist".to_string()) {
      am_types.push("artists");
    }
    if all_types || types.contains(&"playlist".to_string()) {
      am_types.push("playlists");
    }

    let kinds_str = kinds.into_iter().collect::<Vec<_>>().join(",");
    let types_str = am_types.join(",");

    let mut params = vec![
      ("term", query),
      ("extend", "artistUrl"),
      ("kinds", &kinds_str),
    ];

    if !types_str.is_empty() {
      params.push(("types", &types_str));
    }

    let path = format!("/catalog/{}/search/suggestions", self.country_code);
    let mut url = format!("{}{}", API_BASE, path);
    if !params.is_empty() {
      url.push('?');
      for (i, (k, v)) in params.iter().enumerate() {
        if i > 0 {
          url.push('&');
        }
        url.push_str(k);
        url.push('=');
        url.push_str(&urlencoding::encode(v));
      }
    }

    let json = self.api_request(&url).await?;
    let suggestions = json.pointer("/results/suggestions")?.as_array()?;

    let mut tracks = Vec::new();
    let mut albums = Vec::new();
    let mut artists = Vec::new();
    let mut playlists = Vec::new();
    let texts = Vec::new();

    for suggestion in suggestions {
      let kind = suggestion.get("kind").and_then(|v| v.as_str()).unwrap_or("");
      if kind != "terms" {
        let content = suggestion.get("content")?;
        let type_ = content.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let attributes = content.get("attributes")?;
        let url = attributes.get("url").and_then(|v| v.as_str()).unwrap_or("");

        match type_ {
          "songs" => {
            if let Some(track) = self.build_track(content, None) {
              tracks.push(track);
            }
          }
          "albums" => {
            let name = attributes.get("name").and_then(|v| v.as_str()).unwrap_or("Unknown Album");
            let author = attributes.get("artistName").and_then(|v| v.as_str()).unwrap_or("Unknown Artist");
            let artwork = attributes.pointer("/artwork/url").and_then(|v| v.as_str()).map(|s| s.replace("{w}", "500").replace("{h}", "500"));
            let track_count = attributes.get("trackCount").and_then(|v| v.as_u64()).unwrap_or(0);

            albums.push(PlaylistData {
              info: PlaylistInfo {
                name: name.to_string(),
                selected_track: -1,
              },
              plugin_info: serde_json::json!({
                "type": "album",
                "url": url,
                "author": author,
                "artworkUrl": artwork,
                "totalTracks": track_count
              }),
              tracks: Vec::new(),
            });
          }
          "artists" => {
            let name = attributes.get("name").and_then(|v| v.as_str()).unwrap_or("Unknown Artist");
            let artwork = attributes.pointer("/artwork/url").and_then(|v| v.as_str()).map(|s| s.replace("{w}", "500").replace("{h}", "500"));

            artists.push(PlaylistData {
              info: PlaylistInfo {
                name: format!("{}'s Top Tracks", name),
                selected_track: -1,
              },
              plugin_info: serde_json::json!({
                "type": "artist",
                "url": url,
                "author": name,
                "artworkUrl": artwork,
                "totalTracks": 0
              }),
              tracks: Vec::new(),
            });
          }
          "playlists" => {
            let name = attributes.get("name").and_then(|v| v.as_str()).unwrap_or("Unknown Playlist");
            let curator = attributes.get("curatorName").and_then(|v| v.as_str()).unwrap_or("Apple Music");
            let artwork = attributes.pointer("/artwork/url").and_then(|v| v.as_str()).map(|s| s.replace("{w}", "500").replace("{h}", "500"));
            let track_count = attributes.get("trackCount").and_then(|v| v.as_u64()).unwrap_or(0);

            playlists.push(PlaylistData {
              info: PlaylistInfo {
                name: name.to_string(),
                selected_track: -1,
              },
              plugin_info: serde_json::json!({
                "type": "playlist",
                "url": url,
                "author": curator,
                "artworkUrl": artwork,
                "totalTracks": track_count
              }),
              tracks: Vec::new(),
            });
          }
          _ => {}
        }
      }
    }

    Some(crate::api::tracks::SearchResult {
      tracks,
      albums,
      artists,
      playlists,
      texts,
      plugin: serde_json::json!({}),
    })
  }
}
