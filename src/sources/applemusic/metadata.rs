use crate::api::tracks::{LoadResult, PlaylistData, PlaylistInfo};
use super::AppleMusicSource;

impl AppleMusicSource {
  pub(crate) async fn resolve_track(&self, id: &str) -> LoadResult {
    let path = format!("/catalog/{}/songs/{}", self.country_code, id);

    let data = match self.api_request(&path).await {
      Some(d) => d,
      None => return LoadResult::Empty {},
    };

    if let Some(item) = data.pointer("/data/0") {
      if let Some(track) = self.build_track(item, None) {
        return LoadResult::Track(track);
      }
    }
    LoadResult::Empty {}
  }

  pub(crate) async fn resolve_album(&self, id: &str) -> LoadResult {
    let path = format!("/catalog/{}/albums/{}", self.country_code, id);
    let data = match self.api_request(&path).await {
      Some(d) => d,
      None => return LoadResult::Empty {},
    };

    let album = match data.pointer("/data/0") {
      Some(a) => a,
      None => return LoadResult::Empty {},
    };

    let name = album
      .pointer("/attributes/name")
      .and_then(|v| v.as_str())
      .unwrap_or("Unknown Album")
      .to_string();

    let artwork = album
      .pointer("/attributes/artwork/url")
      .and_then(|v| v.as_str())
      .map(|s| s.replace("{w}", "1000").replace("{h}", "1000"));

    let tracks_data = album
      .pointer("/relationships/tracks/data")
      .and_then(|v| v.as_array());

    let mut tracks = Vec::new();
    if let Some(items) = tracks_data {
      for item in items {
        if let Some(track) = self.build_track(item, artwork.clone()) {
          tracks.push(track);
        }
      }
    }

    if tracks.is_empty() {
      return LoadResult::Empty {};
    }

    LoadResult::Playlist(PlaylistData {
      info: PlaylistInfo {
        name,
        selected_track: -1,
      },
      plugin_info: serde_json::json!({
        "type": "album",
        "url": album.pointer("/attributes/url").and_then(|v| v.as_str()),
        "artworkUrl": artwork,
        "author": album.pointer("/attributes/artistName").and_then(|v| v.as_str()),
        "totalTracks": album.pointer("/attributes/trackCount").and_then(|v| v.as_u64()).unwrap_or(tracks.len() as u64)
      }),
      tracks,
    })
  }

  pub(crate) async fn resolve_playlist(&self, id: &str) -> LoadResult {
    let path = format!("/catalog/{}/playlists/{}", self.country_code, id);
    let data = match self.api_request(&path).await {
      Some(d) => d,
      None => return LoadResult::Empty {},
    };

    let playlist = match data.pointer("/data/0") {
      Some(p) => p,
      None => return LoadResult::Empty {},
    };

    let name = playlist
      .pointer("/attributes/name")
      .and_then(|v| v.as_str())
      .unwrap_or("Unknown Playlist")
      .to_string();
    let artwork = playlist
      .pointer("/attributes/artwork/url")
      .and_then(|v| v.as_str())
      .map(|s| s.replace("{w}", "1000").replace("{h}", "1000"));

    let tracks_data = playlist
      .pointer("/relationships/tracks/data")
      .and_then(|v| v.as_array());

    let mut tracks = Vec::new();
    if let Some(items) = tracks_data {
      for item in items {
        if let Some(track) = self.build_track(item, artwork.clone()) {
          tracks.push(track);
        }
      }
    }

    if tracks.is_empty() {
      return LoadResult::Empty {};
    }

    LoadResult::Playlist(PlaylistData {
      info: PlaylistInfo {
        name,
        selected_track: -1,
      },
      plugin_info: serde_json::json!({
        "type": "playlist",
        "url": playlist.pointer("/attributes/url").and_then(|v| v.as_str()),
        "artworkUrl": artwork,
        "author": playlist.pointer("/attributes/curatorName").and_then(|v| v.as_str()),
        "totalTracks": playlist.pointer("/attributes/trackCount").and_then(|v| v.as_u64()).unwrap_or(tracks.len() as u64)
      }),
      tracks,
    })
  }

  pub(crate) async fn resolve_artist(&self, id: &str) -> LoadResult {
    // Fetch top songs
    let path = format!(
      "/catalog/{}/artists/{}/view/top-songs",
      self.country_code, id
    );
    let data = match self.api_request(&path).await {
      Some(d) => d,
      None => return LoadResult::Empty {},
    };

    let tracks_data = data.pointer("/data").and_then(|v| v.as_array());

    // Fetch artist info for name/artwork
    let artist_path = format!("/catalog/{}/artists/{}", self.country_code, id);
    let artist_data = self.api_request(&artist_path).await;

    let (artist_name, artwork) = if let Some(ad) = artist_data {
      let name = ad
        .pointer("/data/0/attributes/name")
        .and_then(|v| v.as_str())
        .unwrap_or("Artist")
        .to_string();
      let art = ad
        .pointer("/data/0/attributes/artwork/url")
        .and_then(|v| v.as_str())
        .map(|s| s.replace("{w}", "1000").replace("{h}", "1000"));
      (name, art)
    } else {
      ("Artist".to_string(), None)
    };

    let mut tracks = Vec::new();
    if let Some(items) = tracks_data {
      for item in items {
        if let Some(track) = self.build_track(item, artwork.clone()) {
          tracks.push(track);
        }
      }
    }

    if tracks.is_empty() {
      return LoadResult::Empty {};
    }

    LoadResult::Playlist(PlaylistData {
      info: PlaylistInfo {
        name: format!("{}'s Top Tracks", artist_name),
        selected_track: -1,
      },
      plugin_info: serde_json::json!({
        "type": "artist",
        "url": format!("https://music.apple.com/artist/{}", id),
        "artworkUrl": artwork,
        "author": artist_name,
        "totalTracks": tracks.len()
      }),
      tracks,
    })
  }
}
