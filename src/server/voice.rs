use tracing::error;

use crate::{
    audio::filters::FilterChain, common::types::Shared, gateway::VoiceGateway,
    player::VoiceConnectionState,
};

pub async fn connect_voice(
    engine: Shared<crate::gateway::VoiceEngine>,
    guild_id: crate::common::types::GuildId,
    user_id: crate::common::types::UserId,
    voice: VoiceConnectionState,
    filter_chain: Shared<FilterChain>,
    ping: std::sync::Arc<std::sync::atomic::AtomicI64>,
    event_tx: Option<tokio::sync::mpsc::UnboundedSender<crate::api::LavalinkEvent>>,
    frames_sent: std::sync::Arc<std::sync::atomic::AtomicU64>,
    frames_nulled: std::sync::Arc<std::sync::atomic::AtomicU64>,
) -> tokio::task::JoinHandle<()> {
    let engine_lock = engine.lock().await;
    let channel_id = voice
        .channel_id
        .as_ref()
        .and_then(|id: &String| id.parse::<u64>().ok())
        .unwrap_or_else(|| guild_id.parse::<u64>().unwrap_or(0));
    let channel_id = crate::common::types::ChannelId(channel_id);

    let mixer = engine_lock.mixer.clone();
    let gateway = VoiceGateway::new(
        guild_id,
        user_id,
        channel_id,
        voice.session_id.into(),
        voice.token,
        voice.endpoint,
        mixer,
        filter_chain,
        ping,
        event_tx,
        frames_sent,
        frames_nulled,
    );

    tokio::spawn(async move {
        if let Err(e) = gateway.run().await {
            error!("Voice gateway error: {}", e);
        }
    })
}
