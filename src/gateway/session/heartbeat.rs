use std::sync::{
    Arc,
    atomic::{AtomicI64, AtomicU32, AtomicU64, Ordering},
};

use tokio::sync::mpsc::UnboundedSender;
use tokio_tungstenite::tungstenite::protocol::Message;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::{
    common::utils::now_ms,
    gateway::{constants::OP_HEARTBEAT, session::types::VoiceGatewayMessage},
};

const JS_SAFE_MAX: u64 = (1u64 << 53) - 1;

pub struct HeartbeatTracker {
    pub last_nonce: Arc<AtomicU64>,
    pub sent_at: Arc<AtomicU64>,
    pub missed_acks: Arc<AtomicU32>,
}

impl Default for HeartbeatTracker {
    fn default() -> Self {
        Self {
            last_nonce: Arc::new(AtomicU64::new(0)),
            sent_at: Arc::new(AtomicU64::new(0)),
            missed_acks: Arc::new(AtomicU32::new(0)),
        }
    }
}

impl HeartbeatTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn validate_ack(&self, acked_nonce: u64) -> Option<u64> {
        let expected = self.last_nonce.load(Ordering::Relaxed);
        if expected != acked_nonce {
            warn!("Heartbeat mismatch: sent={expected} got={acked_nonce}");
            return None;
        }
        Some(now_ms().saturating_sub(self.sent_at.load(Ordering::Relaxed)))
    }

    pub fn spawn(
        &self,
        tx: UnboundedSender<Message>,
        seq_ack: Arc<AtomicI64>,
        conn_token: CancellationToken,
        interval_ms: u64,
    ) -> tokio::task::JoinHandle<()> {
        let last_nonce = self.last_nonce.clone();
        let sent_at = self.sent_at.clone();
        let missed_acks = self.missed_acks.clone();

        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(tokio::time::Duration::from_millis(interval_ms));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            loop {
                ticker.tick().await;

                let missed = missed_acks.fetch_add(1, Ordering::Relaxed);
                if missed >= 2 {
                    warn!("Heartbeat timeout: {missed} missed ACKs. Reconnecting...");
                    conn_token.cancel();
                    break;
                }

                let nonce = rand::random::<u64>() & JS_SAFE_MAX;
                last_nonce.store(nonce, Ordering::Relaxed);
                sent_at.store(now_ms(), Ordering::Relaxed);

                let hb = VoiceGatewayMessage {
                    op: OP_HEARTBEAT,
                    d: serde_json::json!({ "t": nonce, "seq_ack": seq_ack.load(Ordering::Relaxed) }),
                };

                if let Ok(json) = serde_json::to_string(&hb)
                    && tx.send(Message::Text(json.into())).is_err()
                {
                    break;
                }
            }
        })
    }
}
