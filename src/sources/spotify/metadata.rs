use std::sync::Arc;
use futures::future::join_all;
use serde_json::{Value, json};
use tokio::sync::Semaphore;
use crate::api::tracks::{LoadResult, PlaylistData, PlaylistInfo, Track, TrackInfo};
use crate::sources::spotify::token::SpotifyTokenTracker;
use crate::sources::spotify::helpers::SpotifyHelpers;
use crate::sources::spotify::parser::SpotifyParser;

pub struct SpotifyMetadata;

impl SpotifyMetadata {
  pub async fn fetch_metadata_isrc(
    client: &reqwest::Client,
    token_tracker: &Arc<SpotifyTokenTracker>,
    id: &str,
    isrc_binary_regex: &regex::Regex,
  ) -> Option<String> {
    let token = token_tracker.get_token().await?;
    let hex_id = SpotifyHelpers::base62_to_hex(id);
    let url = format!(
      "https://spclient.wg.spotify.com/metadata/4/track/{}?market=from_token",
      hex_id
    );

    let resp = client
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
      if let Some(mat) = isrc_binary_regex.find(&chunk_str) {
        return Some(mat.as_str().to_string());
      }
    }

    // JSON fallback
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

  pub async fn parse_generic_track(
    client: &reqwest::Client,
    token_tracker: &Arc<SpotifyTokenTracker>,
    track_val: &Value,
    artwork_url: Option<String>,
    isrc_binary_regex: &regex::Regex,
  ) -> Option<TrackInfo> {
    let mut track_info = SpotifyParser::parse_track_inner(track_val, artwork_url)?;

    if track_info.isrc.is_none() {
      let isrc = Self::fetch_metadata_isrc(client, token_tracker, &track_info.identifier, isrc_binary_regex).await;
      track_info.isrc = isrc;
    }

    Some(track_info)
  }

  pub async fn fetch_track(
    client: &reqwest::Client,
    token_tracker: &Arc<SpotifyTokenTracker>,
    id: &str,
    isrc_binary_regex: &regex::Regex,
  ) -> Option<TrackInfo> {
    let variables = json!({
        "uri": format!("spotify:track:{}", id)
    });
    let hash = "612585ae06ba435ad26369870deaae23b5c8800a256cd8a57e08eddc25a37294";

    let data = SpotifyHelpers::partner_api_request(client, token_tracker, "getTrack", variables, hash).await?;
    let track = data.pointer("/data/trackUnion")?;
    Self::parse_generic_track(client, token_tracker, track, None, isrc_binary_regex).await
  }

  pub async fn fetch_album(
    client: &reqwest::Client,
    token_tracker: &Arc<SpotifyTokenTracker>,
    id: &str,
    album_load_limit: usize,
    album_page_load_concurrency: usize,
    track_resolve_concurrency: usize,
    isrc_binary_regex: &Arc<regex::Regex>,
  ) -> LoadResult {
    const HASH: &str = "b9bfabef66ed756e5e13f68a942deb60bd4125ec1f1be8cc42769dc0259b4b10";
    const PAGE_LIMIT: u64 = 50;

    let base_vars = json!({
        "uri": format!("spotify:album:{}", id),
        "locale": "en",
        "offset": 0,
        "limit": PAGE_LIMIT
    });

    let data = match SpotifyHelpers::partner_api_request(client, token_tracker, "getAlbum", base_vars.clone(), HASH).await {
      Some(d) => d,
      None => return LoadResult::Empty {},
    };

    let album = match data.pointer("/data/albumUnion") {
      Some(a) => a,
      None => return LoadResult::Empty {},
    };

    let name = album.get("name").and_then(|v| v.as_str()).unwrap_or("Unknown Album").to_string();
    let total_count = album.pointer("/tracksV2/totalCount").and_then(|v| v.as_u64()).unwrap_or(0);
    let album_artwork = album.pointer("/coverArt/sources").and_then(|s| s.as_array()).and_then(|s| s.first()).and_then(|i| i.get("url")).and_then(|v| v.as_str()).map(|s| s.to_string());

    let mut all_items: Vec<Value> = album.pointer("/tracksV2/items").and_then(|i| i.as_array()).cloned().unwrap_or_default();

    if total_count > PAGE_LIMIT {
      let max_tracks = if album_load_limit == 0 { u64::MAX } else { album_load_limit as u64 * PAGE_LIMIT };
      let effective_total = total_count.min(max_tracks);

      if effective_total > PAGE_LIMIT {
        let extra = SpotifyHelpers::fetch_paginated_items(
          client, token_tracker, "getAlbum", HASH, base_vars, "/data/albumUnion/tracksV2/items", effective_total, PAGE_LIMIT, album_page_load_concurrency
        ).await;
        all_items.extend(extra);
      }
    }

    let semaphore = Arc::new(Semaphore::new(track_resolve_concurrency));
    let futs: Vec<_> = all_items.into_iter().take(if album_load_limit > 0 { (PAGE_LIMIT * album_load_limit as u64) as usize } else { usize::MAX }).filter_map(|item| {
        let track_data = item.get("track")?.clone();
        let semaphore = semaphore.clone();
        let artwork = album_artwork.clone();
        let c = client.clone();
        let tt = token_tracker.clone();
        let re = isrc_binary_regex.clone();

        Some(async move {
          let _permit = semaphore.acquire().await.unwrap();
          Self::parse_generic_track(&c, &tt, &track_data, artwork, &re).await
        })
      }).collect();

    let results = join_all(futs).await;
    let tracks: Vec<Track> = results.into_iter().flatten().map(Track::new).collect();

    if tracks.is_empty() {
      LoadResult::Empty {}
    } else {
      LoadResult::Playlist(PlaylistData {
        info: PlaylistInfo { name, selected_track: -1 },
        plugin_info: json!({ "type": "album", "url": format!("https://open.spotify.com/album/{}", id), "artworkUrl": album_artwork, "author": album.pointer("/artists/items/0/profile/name").and_then(|v| v.as_str()), "totalTracks": total_count }),
        tracks,
      })
    }
  }

  pub async fn fetch_playlist(
    client: &reqwest::Client,
    token_tracker: &Arc<SpotifyTokenTracker>,
    id: &str,
    playlist_load_limit: usize,
    playlist_page_load_concurrency: usize,
    track_resolve_concurrency: usize,
    isrc_binary_regex: &Arc<regex::Regex>,
  ) -> LoadResult {
    const HASH: &str = "bb67e0af06e8d6f52b531f97468ee4acd44cd0f82b988e15c2ea47b1148efc77";
    const PAGE_LIMIT: u64 = 100;

    let base_vars = json!({
        "uri": format!("spotify:playlist:{}", id),
        "offset": 0,
        "limit": PAGE_LIMIT,
        "enableWatchFeedEntrypoint": false
    });

    let data = match SpotifyHelpers::partner_api_request(client, token_tracker, "fetchPlaylist", base_vars.clone(), HASH).await {
      Some(d) => d,
      None => return LoadResult::Empty {},
    };

    let playlist = match data.pointer("/data/playlistV2") {
      Some(p) => p,
      None => return LoadResult::Empty {},
    };

    let name = playlist.get("name").and_then(|v| v.as_str()).unwrap_or("Unknown Playlist").to_string();
    let total_count = playlist.pointer("/content/totalCount").and_then(|v| v.as_u64()).unwrap_or(0);
    let mut all_items: Vec<Value> = playlist.pointer("/content/items").and_then(|i| i.as_array()).cloned().unwrap_or_default();

    if total_count > PAGE_LIMIT {
      let max_tracks = if playlist_load_limit == 0 { u64::MAX } else { playlist_load_limit as u64 * PAGE_LIMIT };
      let effective_total = total_count.min(max_tracks);

      if effective_total > PAGE_LIMIT {
        let extra = SpotifyHelpers::fetch_paginated_items(
          client, token_tracker, "fetchPlaylist", HASH, base_vars, "/data/playlistV2/content/items", effective_total, PAGE_LIMIT, playlist_page_load_concurrency
        ).await;
        all_items.extend(extra);
      }
    }

    let semaphore = Arc::new(Semaphore::new(track_resolve_concurrency));
    let futs: Vec<_> = all_items.into_iter().take(if playlist_load_limit > 0 { (PAGE_LIMIT * playlist_load_limit as u64) as usize } else { usize::MAX }).filter_map(|item| {
        let track_data = item.pointer("/item/data").or_else(|| item.pointer("/itemV2/data"))?.clone();
        let semaphore = semaphore.clone();
        let c = client.clone();
        let tt = token_tracker.clone();
        let re = isrc_binary_regex.clone();
        Some(async move {
          let _permit = semaphore.acquire().await.unwrap();
          Self::parse_generic_track(&c, &tt, &track_data, None, &re).await
        })
      }).collect();

    let results = join_all(futs).await;
    let tracks: Vec<Track> = results.into_iter().flatten().map(Track::new).collect();

    if tracks.is_empty() {
      LoadResult::Empty {}
    } else {
      LoadResult::Playlist(PlaylistData {
        info: PlaylistInfo { name: name.clone(), selected_track: -1 },
        plugin_info: json!({
          "type": "playlist",
          "url": format!("https://open.spotify.com/playlist/{}", id),
          "artworkUrl": playlist.pointer("/images/items/0/sources/0/url").and_then(|v| v.as_str()),
          "author": playlist.get("ownerV2").and_then(|v| v.get("name")).and_then(|v| v.as_str()).or_else(|| (id.starts_with("37i9dQZ")).then_some("Spotify")),
          "totalTracks": total_count
        }),
        tracks,
      })
    }
  }

  pub async fn fetch_artist(
    client: &reqwest::Client,
    token_tracker: &Arc<SpotifyTokenTracker>,
    id: &str,
    isrc_binary_regex: &Arc<regex::Regex>,
  ) -> LoadResult {
    let variables = json!({
        "uri": format!("spotify:artist:{}", id),
        "locale": "en",
        "includePrerelease": true
    });
    let hash = "35648a112beb1794e39ab931365f6ae4a8d45e65396d641eeda94e4003d41497";

    let data = match SpotifyHelpers::partner_api_request(client, token_tracker, "queryArtistOverview", variables, hash).await {
      Some(d) => d,
      None => return LoadResult::Empty {},
    };

    let artist = match data.pointer("/data/artistUnion") {
      Some(a) => a,
      None => return LoadResult::Empty {},
    };

    let name = artist.get("profile").and_then(|p| p.get("name")).and_then(|v| v.as_str()).unwrap_or("Unknown Artist").to_string();
    let mut tracks = Vec::new();

    if let Some(items) = artist.pointer("/discography/topTracks/items").and_then(|i| i.as_array()) {
      for item in items {
        if let Some(track_data) = item.get("track") {
          let c = client.clone();
          let tt = token_tracker.clone();
          let re = isrc_binary_regex.clone();
          if let Some(track_info) = Self::parse_generic_track(&c, &tt, track_data, None, &re).await {
            tracks.push(Track::new(track_info));
          }
        }
      }
    }

    if tracks.is_empty() {
      LoadResult::Empty {}
    } else {
      LoadResult::Playlist(PlaylistData {
        info: PlaylistInfo { name: name.clone(), selected_track: -1 },
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
