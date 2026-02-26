use std::sync::{
    Arc,
    atomic::{AtomicI64, AtomicU64},
};

use tracing::error;

use crate::{
    api::LavalinkEvent,
    audio::filters::FilterChain,
    common::types::{ChannelId, GuildId, Shared, UserId},
    gateway::{VoiceEngine, VoiceGateway},
    player::VoiceConnectionState,
};

/// Spawns the voice gateway task for the given guild.
///
/// Returns a `JoinHandle` so the caller can abort the task on disconnect.
///
/// # Errors (via tracing)
/// Any error from `VoiceGateway::run` is logged and the task exits silently.
pub async fn connect_voice(
    engine: Shared<VoiceEngine>,
    guild_id: GuildId,
    user_id: UserId,
    voice: VoiceConnectionState,
    filter_chain: Shared<FilterChain>,
    ping: Arc<AtomicI64>,
    event_tx: Option<tokio::sync::mpsc::UnboundedSender<LavalinkEvent>>,
    frames_sent: Arc<AtomicU64>,
    frames_nulled: Arc<AtomicU64>,
) -> tokio::task::JoinHandle<()> {
    // `channel_id` is required — parse it from the voice state.
    let channel_id = voice
        .channel_id
        .as_deref()
        .and_then(|id| id.parse::<u64>().ok())
        .map(ChannelId)
        .expect("channel_id is required to connect voice");

    // Narrow the lock scope: release the guard as soon as we have the mixer.
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
