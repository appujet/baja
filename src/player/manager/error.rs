use super::super::context::PlayerContext;
use crate::{
    protocol::{
        self,
        events::{RustalinkEvent, TrackEndReason, TrackException},
    },
    server::Session,
};

/// Emit `TrackException` followed by `TrackEnd` to indicate a track load failure.
///
/// If the player has no current track, the function returns without sending any events.
/// When a track is present, this sends a `TrackException` event containing the provided
/// message (as the exception message, cause, and cause stack trace) with severity `Common`,
/// and then a `TrackEnd` event with reason `LoadFailed`.
///
/// # Examples
///
/// ```
/// # use crate::{PlayerContext, Session};
/// # // The following is an illustrative example; replace with real PlayerContext and Session.
/// # async fn example(player: &PlayerContext, session: &Session) {
/// send_load_failed(player, session, "failed to load track".to_string()).await;
/// # }
/// ```
pub async fn send_load_failed(player: &PlayerContext, session: &Session, message: String) {
    let Some(track) = player.to_player_response().await.track else {
        return;
    };
    let guild_id = player.guild_id.clone();

    session.send_message(&protocol::OutgoingMessage::Event {
        event: Box::new(RustalinkEvent::TrackException {
            guild_id: guild_id.clone(),
            track: track.clone(),
            exception: TrackException {
                message: Some(message.clone()),
                severity: crate::common::Severity::Common,
                cause: message.clone(),
                cause_stack_trace: Some(message),
            },
        }),
    });

    session.send_message(&protocol::OutgoingMessage::Event {
        event: Box::new(RustalinkEvent::TrackEnd {
            guild_id,
            track,
            reason: TrackEndReason::LoadFailed,
        }),
    });
}
