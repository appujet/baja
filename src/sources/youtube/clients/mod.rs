use crate::common::types::AnyResult;
pub mod android;
pub mod android_vr;
pub mod common;
pub mod ios;
pub mod music_android;
pub mod tv;
pub mod tv_cast;
pub mod tv_embedded;
pub mod web;
pub mod web_embedded;
pub mod web_parent_tools;
pub mod web_remix;

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use super::{cipher::YouTubeCipherManager, oauth::YouTubeOAuth, sabr::SabrConfig};
use crate::protocol::tracks::Track;

#[async_trait]
pub trait YouTubeClient: Send + Sync {
    fn name(&self) -> &str;
    fn client_name(&self) -> &str;
    fn client_version(&self) -> &str;
    fn user_agent(&self) -> &str;

    async fn search(
        &self,
        query: &str,
        context: &Value,
        oauth: Arc<YouTubeOAuth>,
    ) -> AnyResult<Vec<Track>>;
    async fn get_track_info(
        &self,
        track_id: &str,
        context: &Value,
        oauth: Arc<YouTubeOAuth>,
    ) -> AnyResult<Option<Track>>;
    async fn resolve_url(
        &self,
        url: &str,
        context: &Value,
        oauth: Arc<YouTubeOAuth>,
    ) -> AnyResult<Option<Track>>;
    async fn get_track_url(
        &self,
        track_id: &str,
        context: &Value,
        cipher_manager: Arc<YouTubeCipherManager>,
        oauth: Arc<YouTubeOAuth>,
    ) -> AnyResult<Option<String>>;
    async fn get_playlist(
        &self,
        playlist_id: &str,
        context: &Value,
        oauth: Arc<YouTubeOAuth>,
    ) -> AnyResult<Option<(Vec<Track>, String)>>;
    /// Fetch the raw `streamingData` JSON from the player API for stream URL resolution.
    /// Returns `None` if the video is not playable or this client doesn't support it.
    /// The WEB client must NOT override this (it uses SABR protocol instead).
    async fn get_streaming_data(
        &self,
        _track_id: &str,
        _context: &Value,
        _cipher_manager: Arc<YouTubeCipherManager>,
        _oauth: Arc<YouTubeOAuth>,
    ) -> AnyResult<Option<Value>> {
        Ok(None)
    }

    /// Try to fetch a SABR config for this client. Default: returns `None`.
    /// Only the WEB client overrides this with a real implementation.
    async fn get_sabr_config(
        &self,
        _track_id: &str,
        _visitor_data: Option<&str>,
        _signature_timestamp: Option<u32>,
        _cipher_manager: Arc<YouTubeCipherManager>,
        _start_time_ms: u64,
    ) -> Option<SabrConfig> {
        None
    }
}
