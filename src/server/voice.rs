use std::sync::{
    Arc,
    atomic::{AtomicI64, AtomicU64},
};

use tracing::error;

use crate::{
    audio::filters::FilterChain,
    common::types::{ChannelId, GuildId, Shared, UserId},
    gateway::{VoiceEngine, VoiceGateway},
    player::VoiceConnectionState,
    protocol::RustalinkEvent,
};

/// Spawns the voice gateway task for the given guild.
pub async fn connect_voice(
    engine: Shared<VoiceEngine>,
    guild_id: GuildId,
    user_id: UserId,
    voice: VoiceConnectionState,
    filter_chain: Shared<FilterChain>,
    ping: Arc<AtomicI64>,
    event_tx: Option<tokio::sync::mpsc::UnboundedSender<RustalinkEvent>>,
    frames_sent: Arc<AtomicU64>,
    frames_nulled: Arc<AtomicU64>,
) -> tokio::task::JoinHandle<()> {
    let channel_id = match voice
        .channel_id
        .as_deref()
        .and_then(|id| id.parse::<u64>().ok())
        .map(ChannelId)
    {
        Some(id) => id,
        None => {
            error!("Failed to connect voice: channel_id is missing or invalid");
            return tokio::spawn(async {});
        }
    };

    let mixer = {
        let engine_lock = engine.lock().await;
        engine_lock.mixer.clone()
    };

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
