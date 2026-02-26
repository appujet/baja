use crate::gateway::session::types::VoiceGatewayMessage;
use std::sync::{
    Arc,
    atomic::{AtomicI64, AtomicU64, Ordering},
};
use tokio_tungstenite::tungstenite::protocol::Message;

pub fn spawn_heartbeat(
    tx_hb: tokio::sync::mpsc::UnboundedSender<Message>,
    seq_ack: Arc<AtomicI64>,
    last_heartbeat: Arc<AtomicU64>,
    interval_ms: u64,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(interval_ms));
        loop {
            interval.tick().await;
            let current_seq = seq_ack.load(Ordering::Relaxed);
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64;

            last_heartbeat.store(now_ms, Ordering::Relaxed);

            let hb = VoiceGatewayMessage {
                op: 3,
                d: serde_json::json!({
                    "t": now_ms,
                    "seq_ack": current_seq
                }),
            };
            if let Ok(json) = serde_json::to_string(&hb) {
                if tx_hb.send(Message::Text(json.into())).is_err() {
                    break; // Channel closed — session ending
                }
            }
        }
    })
}
