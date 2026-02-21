/// Request parameters for the `loadtracks` endpoint.
#[derive(serde::Deserialize)]
pub struct LoadTracksQuery {
  /// The identifier/link to load.
  pub identifier: String,
}

/// Request parameters for the `loadsearch` endpoint.
#[derive(serde::Deserialize)]
pub struct LoadSearchQuery {
  /// The search query
  pub query: String,
  /// Comma-separated list of types to search for (e.g. "track,playlist,album,artist,text")
  pub types: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DecodeTrackQuery {
  pub encoded_track: Option<String>,
  pub track: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct EncodedTracks {
  pub tracks: Vec<String>,
}

#[derive(serde::Serialize)]
pub struct Exception {
  pub message: String,
  pub severity: String,
  pub cause: String,
}
