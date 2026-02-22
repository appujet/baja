use regex::Regex;
use crate::api::tracks::{LoadResult, PlaylistData, PlaylistInfo};
use super::JioSaavnSource;
use super::helpers::get_json;
use super::parser::parse_track;

impl JioSaavnSource {
  pub async fn get_recommendations(&self, query: &str) -> LoadResult {
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

    let station_id = get_json(&self.client, &params).await.and_then(|json| {
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

      if let Some(json) = get_json(&self.client, &params).await {
        if let Some(obj) = json.as_object() {
          let tracks: Vec<_> = obj
            .values()
            .filter_map(|v| v.get("song"))
            .filter_map(|song| parse_track(song))
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

        if let Some(json) = get_json(&self.client, &params).await {
          if let Some(arr) = json.as_array() {
            let tracks: Vec<_> = arr
              .iter()
              .take(self.recommendations_limit)
              .filter_map(|item| parse_track(item))
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
}
