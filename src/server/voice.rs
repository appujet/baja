use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::error;
use crate::server::UserId;
use crate::player::VoiceConnectionState;
use crate::voice::VoiceGateway;

pub async fn connect_voice(
    engine: Arc<Mutex<crate::voice::VoiceEngine>>,
    guild_id: String,
    user_id: UserId,
    voice: VoiceConnectionState,
) -> tokio::task::JoinHandle<()> {
    let engine_lock = engine.lock().await;
    let channel_id = voice
        .channel_id
        .as_ref()
        .and_then(|id: &String| id.parse::<u64>().ok())
        .unwrap_or_else(|| guild_id.parse::<u64>().unwrap_or(0));

    let mixer = engine_lock.mixer.clone();
    let gateway = VoiceGateway::new(
        guild_id,
        user_id,
        channel_id,
        voice.session_id,
        voice.token,
        voice.endpoint,
        mixer,
    );

    tokio::spawn(async move {
        if let Err(e) = gateway.run().await {
            error!("Voice gateway error: {}", e);
        }
    })
}
