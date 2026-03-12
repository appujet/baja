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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_heartbeat_tracker_new() {
        let tracker = HeartbeatTracker::new();
        assert_eq!(tracker.last_nonce.load(Ordering::Relaxed), 0);
        assert_eq!(tracker.sent_at.load(Ordering::Relaxed), 0);
        assert_eq!(tracker.missed_acks.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_heartbeat_tracker_default() {
        let tracker = HeartbeatTracker::default();
        assert_eq!(tracker.last_nonce.load(Ordering::Relaxed), 0);
        assert_eq!(tracker.sent_at.load(Ordering::Relaxed), 0);
        assert_eq!(tracker.missed_acks.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_validate_ack_matching() {
        let tracker = HeartbeatTracker::new();
        let nonce = 12345u64;

        tracker.last_nonce.store(nonce, Ordering::Relaxed);
        tracker.sent_at.store(now_ms(), Ordering::Relaxed);

        let result = tracker.validate_ack(nonce);
        assert!(result.is_some());

        let rtt = result.unwrap();
        assert!(rtt < 1000); // Should be very small for local test
    }

    #[test]
    fn test_validate_ack_mismatch() {
        let tracker = HeartbeatTracker::new();

        tracker.last_nonce.store(12345, Ordering::Relaxed);
        tracker.sent_at.store(now_ms(), Ordering::Relaxed);

        let result = tracker.validate_ack(99999);
        assert!(result.is_none());
    }

    #[test]
    fn test_validate_ack_zero_nonce() {
        let tracker = HeartbeatTracker::new();

        tracker.last_nonce.store(0, Ordering::Relaxed);
        tracker.sent_at.store(now_ms(), Ordering::Relaxed);

        let result = tracker.validate_ack(0);
        assert!(result.is_some());
    }

    #[test]
    fn test_validate_ack_rtt_calculation() {
        let tracker = HeartbeatTracker::new();
        let nonce = 12345u64;

        // Simulate a heartbeat sent 100ms ago
        let sent_time = now_ms().saturating_sub(100);
        tracker.last_nonce.store(nonce, Ordering::Relaxed);
        tracker.sent_at.store(sent_time, Ordering::Relaxed);

        let result = tracker.validate_ack(nonce);
        assert!(result.is_some());

        let rtt = result.unwrap();
        assert!(rtt >= 100);
        assert!(rtt < 200); // Allow some tolerance
    }

    #[test]
    fn test_missed_acks_counter() {
        let tracker = HeartbeatTracker::new();

        assert_eq!(tracker.missed_acks.load(Ordering::Relaxed), 0);

        tracker.missed_acks.fetch_add(1, Ordering::Relaxed);
        assert_eq!(tracker.missed_acks.load(Ordering::Relaxed), 1);

        tracker.missed_acks.fetch_add(1, Ordering::Relaxed);
        assert_eq!(tracker.missed_acks.load(Ordering::Relaxed), 2);

        tracker.missed_acks.store(0, Ordering::Relaxed);
        assert_eq!(tracker.missed_acks.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_atomic_updates() {
        let tracker = HeartbeatTracker::new();

        tracker.last_nonce.store(100, Ordering::Relaxed);
        tracker.sent_at.store(200, Ordering::Relaxed);
        tracker.missed_acks.store(3, Ordering::Relaxed);

        assert_eq!(tracker.last_nonce.load(Ordering::Relaxed), 100);
        assert_eq!(tracker.sent_at.load(Ordering::Relaxed), 200);
        assert_eq!(tracker.missed_acks.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn test_validate_ack_multiple_times() {
        let tracker = HeartbeatTracker::new();

        for i in 1..=5 {
            let nonce = i * 1000;
            tracker.last_nonce.store(nonce, Ordering::Relaxed);
            tracker.sent_at.store(now_ms(), Ordering::Relaxed);

            let result = tracker.validate_ack(nonce);
            assert!(result.is_some(), "Iteration {}", i);
        }
    }
}
