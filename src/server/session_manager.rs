use axum::extract::ws::Message;
use dashmap::DashMap;

use crate::{
  api,
  common::types::{GuildId, SessionId, UserId},
  player::PlayerContext,
};

/// Alias for the player registry within a session.
pub type PlayerMap = DashMap<GuildId, PlayerContext>;

/// client session.
pub struct Session {
  pub session_id: SessionId,
  pub user_id: Option<UserId>,
  pub players: PlayerMap,
  /// Sender for outgoing WS messages. Swapped on resume.
  pub sender: tokio::sync::Mutex<flume::Sender<Message>>,
  pub resumable: std::sync::atomic::AtomicBool,
  pub resume_timeout: std::sync::atomic::AtomicU64,
  /// True when WS is disconnected but session is kept for resume.
  pub paused: std::sync::atomic::AtomicBool,
  /// Events queued while session is paused.
  pub event_queue: tokio::sync::Mutex<Vec<String>>,
}

impl Session {
  /// Send a JSON message. If paused, queue it for replay.
  pub async fn send_json(&self, json: &str) {
    if self.paused.load(std::sync::atomic::Ordering::Relaxed) {
      let mut queue = self.event_queue.lock().await;
      if queue.len() >= 1000 {
        queue.remove(0); // Drop oldest event if queue is too large
      }
      queue.push(json.to_string());
    } else {
      let sender = self.sender.lock().await;
      let msg = Message::Text(json.to_string().into());
      let _ = sender.send(msg);
    }
  }

  /// Send a typed outgoing message.
  pub async fn send_message(&self, msg: &api::OutgoingMessage) {
    if let Ok(json) = serde_json::to_string(msg) {
      self.send_json(&json).await;
    }
  }

  pub fn shutdown(&self) {
    tracing::info!("Shutting down session: {}", self.session_id);
    for item in self.players.iter() {
      let player = item.value();
      if let Some(task) = &player.gateway_task {
        task.abort();
      }
    }
    self.players.clear();
  }
}

impl Drop for Session {
  fn drop(&mut self) {
    tracing::info!("Dropping session: {}", self.session_id);
    for item in self.players.iter() {
      let player = item.value();
      if let Some(task) = &player.gateway_task {
        task.abort();
      }
    }
    self.players.clear();
  }
}
