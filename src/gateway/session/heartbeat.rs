use std::sync::{
    Arc,
    atomic::{AtomicI64, AtomicU32, AtomicU64, Ordering},
};

use tokio::sync::mpsc::UnboundedSender;
use tokio_tungstenite::tungstenite::protocol::Message;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use super::protocol::{GatewayPayload, OpCode};
use crate::common::utils::now_ms;

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

    /// Spawns a background task that sends periodic heartbeat payloads over the provided WebSocket sender
    /// and tracks missed heartbeat acknowledgements, cancelling the connection token on timeout.
    ///
    /// The spawned task:
    /// - sends a heartbeat every `interval_ms` milliseconds with a monotonic nonce and the current `seq_ack`,
    /// - increments an internal missed-ACK counter each tick and cancels `conn_token` when missed ACKs reach 2,
    /// - updates internal `last_nonce` and `sent_at` timestamps for RTT measurement.
    ///
    /// # Parameters
    ///
    /// - `tx`: WebSocket message sender used to transmit heartbeat `Text` messages.
    /// - `seq_ack`: shared atomic holding the last acknowledged sequence number to include in the heartbeat payload.
    /// - `conn_token`: cancellation token that will be cancelled when heartbeat timeouts occur.
    /// - `interval_ms`: heartbeat interval in milliseconds.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::Arc;
    /// use tokio::sync::mpsc::unbounded_channel;
    /// use std::sync::atomic::AtomicI64;
    /// use tokio_util::sync::CancellationToken;
    ///
    /// // create tracker and channel
    /// let tracker = crate::gateway::session::heartbeat::HeartbeatTracker::new();
    /// let (tx, _rx) = unbounded_channel();
    /// let seq_ack = Arc::new(AtomicI64::new(0));
    /// let token = CancellationToken::new();
    ///
    /// // spawn the heartbeat task (runs in background)
    /// let _handle = tracker.spawn(tx, seq_ack, token, 1_000);
    /// ```
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
                    warn!("Heartbeat timeout: {missed} missed ACKs.");
                    conn_token.cancel();
                    break;
                }

                let nonce = now_ms();
                last_nonce.store(nonce, Ordering::Relaxed);
                sent_at.store(nonce, Ordering::Relaxed);

                let hb = GatewayPayload {
                    op: OpCode::Heartbeat as u8,
                    seq: None,
                    d: serde_json::json!({
                        "t": nonce,
                        "seq_ack": seq_ack.load(Ordering::Relaxed)
                    }),
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
