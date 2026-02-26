use super::super::context::PlayerContext;
use crate::{
    api::{
        self,
        events::{RustalinkEvent, TrackEndReason, TrackException},
    },
    server::Session,
};

/// Emit `TrackException` followed by `TrackEnd: LoadFailed`.
pub async fn send_load_failed(player: &PlayerContext, session: &Session, message: String) {
    let Some(track) = player.to_player_response().track else {
        return;
    };
    let guild_id = player.guild_id.clone();

    session.send_message(&api::OutgoingMessage::Event(
        RustalinkEvent::TrackException {
            guild_id: guild_id.clone(),
            track: track.clone(),
            exception: TrackException {
                message: Some(message.clone()),
                severity: crate::common::Severity::Common,
                cause: message.clone(),
                cause_stack_trace: Some(message),
            },
        },
    ));

    session.send_message(&api::OutgoingMessage::Event(RustalinkEvent::TrackEnd {
        guild_id,
        track,
        reason: TrackEndReason::LoadFailed,
    }));
}
