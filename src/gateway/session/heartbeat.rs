use std::sync::{
    Arc,
    atomic::{AtomicI64, AtomicU64, Ordering},
};
use tokio_tungstenite::tungstenite::protocol::Message;

use crate::{
    common::utils::now_ms,
    gateway::{constants::OP_HEARTBEAT, session::types::VoiceGatewayMessage},
};

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
            let now = now_ms();

            last_heartbeat.store(now, Ordering::Relaxed);

            let hb = VoiceGatewayMessage {
                op: OP_HEARTBEAT,
                d: serde_json::json!({
                    "t": now,
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
