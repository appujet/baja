/// Request parameters for the `loadtracks` endpoint.
#[derive(serde::Deserialize)]
pub struct LoadTracksQuery {
    /// The identifier/link to load.
    pub identifier: String,
}

#[derive(serde::Serialize)]
pub struct Exception {
    pub message: String,
    pub severity: String,
    pub cause: String,
}
