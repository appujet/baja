use crate::common::types::AnyResult;
pub mod android;
pub mod android_vr;
pub mod common;
pub mod ios;
pub mod music_android;
pub mod tv;
pub mod web;
pub mod web_embedded;
pub mod web_remix;

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use super::{cipher::YouTubeCipherManager, oauth::YouTubeOAuth};
use crate::api::tracks::Track;

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
}
