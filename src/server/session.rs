use crate::player::PlayerContext;
use crate::types;
use dashmap::DashMap;
use tokio::sync::Mutex;
use axum::extract::ws::Message;

pub type UserId = u64;

/// A single client session.
pub struct Session {
    pub session_id: String,
    pub user_id: Option<UserId>,
    pub players: DashMap<String, PlayerContext>,
    /// Sender for outgoing WS messages. Swapped on resume.
    pub sender: Mutex<flume::Sender<Message>>,
    pub resumable: std::sync::atomic::AtomicBool,
    pub resume_timeout: std::sync::atomic::AtomicU64,
    /// True when WS is disconnected but session is kept for resume.
    pub paused: std::sync::atomic::AtomicBool,
    /// Events queued while session is paused.
    pub event_queue: Mutex<Vec<String>>,
}

impl Session {
    /// Send a JSON message. If paused, queue it for replay.
    pub async fn send_json(&self, json: &str) {
        if self.paused.load(std::sync::atomic::Ordering::Relaxed) {
            let mut queue = self.event_queue.lock().await;
            queue.push(json.to_string());
        } else {
            let sender = self.sender.lock().await;
            let msg = Message::Text(json.to_string().into());
            let _ = sender.send(msg);
        }
    }

    /// Send a typed outgoing message.
    pub async fn send_message(&self, msg: &types::OutgoingMessage) {
        if let Ok(json) = serde_json::to_string(msg) {
            self.send_json(&json).await;
        }
    }
}
