/// Request parameters for the `loadtracks` endpoint.
#[derive(serde::Deserialize)]
pub struct LoadTracksQuery {
  /// The identifier/link to load.
  pub identifier: String,
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
