use serde_json::Value;
use super::DeezerSource;
use super::PUBLIC_API_BASE;

impl DeezerSource {
  pub(crate) async fn get_json_public(&self, path: &str) -> Option<Value> {
    let url = format!("{}/{}", PUBLIC_API_BASE, path);
    self.client.get(&url).send().await.ok()?.json().await.ok()
  }
}
