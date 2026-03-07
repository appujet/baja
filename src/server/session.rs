use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use axum::extract::ws::Message;
use dashmap::DashMap;
use parking_lot::Mutex;
use tokio::task::AbortHandle;

use crate::{
    common::types::{GuildId, SessionId, UserId},
    player::PlayerContext,
    protocol,
    server::AppState,
};

pub type PlayerMap = DashMap<GuildId, Arc<tokio::sync::RwLock<PlayerContext>>>;

pub struct Session {
    pub session_id: SessionId,
    pub user_id: Option<UserId>,
    pub players: PlayerMap,
    pub sender: Mutex<flume::Sender<Message>>,
    pub resumable: AtomicBool,
    pub resume_timeout: AtomicU64,
    /// True when WS is disconnected but session is kept for resume.
    pub paused: AtomicBool,
    pub event_queue: Mutex<VecDeque<String>>,
    pub max_queue_size: usize,

    pub last_stats_sent: AtomicU64,
    pub last_stats_nulled: AtomicU64,
    pub total_sent_historical: AtomicU64,
    pub total_nulled_historical: AtomicU64,

    /// Abort handles for all spawned player tasks (gateway + track).
    /// Stored separately so shutdown never needs to hold the player write lock.
    task_handles: Mutex<Vec<AbortHandle>>,
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
            task_handles: Mutex::new(Vec::new()),
        }
    }

    /// Register an abort handle for a spawned player task.
    ///
    /// Called by player manager code when it spawns the gateway or track task.
    /// The handle is aborted during session shutdown without needing the player lock.
    pub fn register_task(&self, handle: AbortHandle) {
        self.task_handles.lock().push(handle);
    }

    pub fn get_or_create_player(
        &self,
        guild_id: GuildId,
        state: Arc<AppState>,
    ) -> Arc<tokio::sync::RwLock<PlayerContext>> {
        self.players
            .entry(guild_id.clone())
            .or_insert_with(|| {
                state.player_created();
                Arc::new(tokio::sync::RwLock::new(PlayerContext::new(
                    guild_id,
                    &state.config.player,
                    state.clone(),
                )))
            })
            .value()
            .clone()
    }

    pub async fn destroy_player(&self, guild_id: &GuildId, state: &AppState) -> bool {
        if let Some((_, player_arc)) = self.players.remove(guild_id) {
            let mut player = player_arc.write().await;
            player.destroy().await;
            state.player_destroyed();
            true
        } else {
            false
        }
    }

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

    pub fn send_message(&self, msg: &protocol::OutgoingMessage) {
        if let Ok(json) = serde_json::to_string(msg) {
            self.send_json(json);
        }
    }

    pub async fn shutdown(&self, state: &AppState) {
        tracing::info!("Shutting down session: {}", self.session_id);
        self.stop_all_players();
        let guilds: Vec<GuildId> = self.players.iter().map(|kv| kv.key().clone()).collect();
        for guild in guilds {
            self.destroy_player(&guild, state).await;
        }
    }

    fn stop_all_players(&self) {
        for handle in self.task_handles.lock().drain(..) {
            handle.abort();
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
