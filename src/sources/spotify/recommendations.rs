use std::sync::Arc;
use futures::future::join_all;
use serde_json::{Value, json};
use crate::api::tracks::{LoadResult, PlaylistData, PlaylistInfo, Track};
use crate::sources::spotify::token::SpotifyTokenTracker;
use crate::sources::spotify::helpers::SpotifyHelpers;
use crate::sources::spotify::parser::SpotifyParser;
use crate::sources::spotify::search::SpotifySearch;

pub struct SpotifyRecommendations;

impl SpotifyRecommendations {
  pub async fn fetch_recommendations(
    client: &reqwest::Client,
    token_tracker: &Arc<SpotifyTokenTracker>,
    query: &str,
    mix_regex: &regex::Regex,
    recommendations_limit: usize,
    search_limit: usize,
  ) -> Result<LoadResult, String> {
    let mut seed = query.to_string();

    if let Some(caps) = mix_regex.captures(query) {
      let mut seed_type = caps.get(1).unwrap().as_str().to_string();
      seed = caps.get(2).unwrap().as_str().to_string();

      if seed_type == "isrc" {
        if let Some(res) = SpotifySearch::search_full(client, token_tracker, &format!("isrc:{}", seed), &["track".to_string()], search_limit).await {
          if let Some(track) = res.tracks.first() {
            seed = track.info.identifier.clone();
            seed_type = "track".to_string();
          } else {
            return Ok(LoadResult::Empty {});
          }
        } else {
          return Ok(LoadResult::Empty {});
        }
      }

      let token = match token_tracker.get_token().await {
        Some(t) => t,
        None => return Ok(LoadResult::Empty {}),
      };

      let url = format!(
        "https://spclient.wg.spotify.com/inspiredby-mix/v2/seed_to_playlist/spotify:{}:{}?response-format=json",
        seed_type, seed
      );

      let resp = client
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
                return Err(id.to_string());
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
    Ok(Self::fetch_pathfinder_recommendations(client, token_tracker, track_id, recommendations_limit).await)
  }

  pub async fn fetch_pathfinder_recommendations(
    client: &reqwest::Client,
    token_tracker: &Arc<SpotifyTokenTracker>,
    id: &str,
    recommendations_limit: usize,
  ) -> LoadResult {
    let variables = json!({
        "uri": format!("spotify:track:{}", id),
        "limit": recommendations_limit
    });
    let hash = "c77098ee9d6ee8ad3eb844938722db60570d040b49f41f5ec6e7be9160a7c86b";

    let data = match SpotifyHelpers::partner_api_request(client, token_tracker, "internalLinkRecommenderTrack", variables, hash).await {
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
      .map(|item| async move { SpotifyParser::parse_track_inner(&item, None) })
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

    tracks.truncate(recommendations_limit);

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
}
