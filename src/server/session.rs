use std::{
    collections::VecDeque,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

use axum::extract::ws::Message;
use dashmap::DashMap;
use parking_lot::Mutex;

use crate::{
    common::types::{GuildId, SessionId, UserId},
    player::PlayerContext,
    protocol,
};

/// Alias for the player registry within a session.
pub type PlayerMap = DashMap<GuildId, std::sync::Arc<tokio::sync::RwLock<PlayerContext>>>;

/// A client session managing multiple players and WebSocket communication.
pub struct Session {
    pub session_id: SessionId,
    pub user_id: Option<UserId>,
    pub players: PlayerMap,
    /// Sender for outgoing WS messages.
    pub sender: Mutex<flume::Sender<Message>>,
    pub resumable: AtomicBool,
    pub resume_timeout: AtomicU64,
    /// True when WS is disconnected but session is kept for resume.
    pub paused: AtomicBool,
    /// Events queued while session is paused.
    pub event_queue: Mutex<VecDeque<String>>,
    pub max_queue_size: usize,

    /// Last recorded frames sent for stats calculation.
    pub last_stats_sent: AtomicU64,
    /// Last recorded frames nulled for stats calculation.
    pub last_stats_nulled: AtomicU64,

    /// Historical total frames sent (from closed players).
    pub total_sent_historical: AtomicU64,
    /// Historical total frames nulled (from closed players).
    pub total_nulled_historical: AtomicU64,
}

impl Session {
    pub fn new(
        session_id: SessionId,
        user_id: Option<UserId>,
        sender: flume::Sender<Message>,
        max_queue_size: usize,
    ) -> Self {
        Self {
            session_id,
            user_id,
            players: DashMap::new(),
            sender: Mutex::new(sender),
            resumable: AtomicBool::new(false),
            resume_timeout: AtomicU64::new(60),
            paused: AtomicBool::new(false),
            event_queue: Mutex::new(VecDeque::new()),
            max_queue_size,
            last_stats_sent: AtomicU64::new(0),
            last_stats_nulled: AtomicU64::new(0),
            total_sent_historical: AtomicU64::new(0),
            total_nulled_historical: AtomicU64::new(0),
        }
    }

    /// Sends a JSON message. If paused, queues it for replay.
    pub fn send_json(&self, json: impl Into<String>) {
        if self.paused.load(Ordering::Relaxed) {
            let mut queue = self.event_queue.lock();
            if queue.len() >= self.max_queue_size {
                queue.pop_front();
            }
            queue.push_back(json.into());
        } else {
            let sender = self.sender.lock().clone();
            let msg = Message::Text(json.into().into());
            let _ = sender.send(msg);
        }
    }

    /// Sends a typed outgoing message.
    pub fn send_message(&self, msg: &protocol::OutgoingMessage) {
        if let Ok(json) = serde_json::to_string(msg) {
            self.send_json(json);
        }
    }

    /// Shuts down all players in this session.
    pub fn shutdown(&self) {
        tracing::info!("Shutting down session: {}", self.session_id);
        self.stop_all_players();
        self.players.clear();
    }

    fn stop_all_players(&self) {
        let players: Vec<_> = self
            .players
            .iter()
            .map(|item| item.value().clone())
            .collect();
        for player_arc in players {
            if let Ok(player) = player_arc.try_write() {
                if let Some(task) = &player.gateway_task {
                    task.abort();
                }
                if let Some(task) = &player.track_task {
                    task.abort();
                }
            }
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        tracing::info!("Dropping session: {}", self.session_id);
        self.stop_all_players();
        self.players.clear();
    }
}
