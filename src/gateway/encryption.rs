use std::{
    collections::{HashMap, HashSet},
    num::NonZeroU16,
};

use davey::{DaveSession, ProposalsOperationType};
use tracing::{debug, info, warn};

use crate::{
    common::types::{AnyError, AnyResult},
    gateway::{
        constants::{DAVE_INITIAL_VERSION, MAX_PENDING_PROPOSALS, SILENCE_FRAME},
        session::types::map_boxed_err,
    },
};

/// Minimum NonZeroU16 version (1) used for DAVE session init.
const DAVE_MIN_VERSION: NonZeroU16 = match NonZeroU16::new(DAVE_INITIAL_VERSION) {
    Some(v) => v,
    None => unreachable!(),
};

pub struct DaveHandler {
    session: Option<DaveSession>,
    user_id: crate::common::types::UserId,
    channel_id: crate::common::types::ChannelId,
    protocol_version: u16,
    pending_transitions: HashMap<u16, u16>,
    external_sender_set: bool,
    pending_proposals: Vec<Vec<u8>>,
    was_ready: bool,
}

impl DaveHandler {
    pub fn new(
        user_id: crate::common::types::UserId,
        channel_id: crate::common::types::ChannelId,
    ) -> Self {
        Self {
            session: None,
            user_id,
            channel_id,
            protocol_version: 0,
            pending_transitions: HashMap::new(),
            external_sender_set: false,
            pending_proposals: Vec::new(),
            was_ready: false,
        }
    }

    pub fn setup_session(&mut self, version: u16) -> AnyResult<Vec<u8>> {
        self.protocol_version = version;
        let nz_version = NonZeroU16::new(version).unwrap_or(DAVE_MIN_VERSION);

        let session = if let Some(s) = &mut self.session {
            s.reinit(nz_version, self.user_id.0, self.channel_id.0, None)
                .map_err(map_boxed_err)?;
            s
        } else {
            self.session = Some(
                DaveSession::new(nz_version, self.user_id.0, self.channel_id.0, None)
                    .map_err(map_boxed_err)?,
            );
            self.session.as_mut().expect("just inserted")
        };

        // Reset handshake state on every (re)init.
        self.external_sender_set = false;
        self.pending_proposals.clear();
        self.was_ready = false;

        let key_package = session.create_key_package().map_err(map_boxed_err)?;
        debug!("DAVE session setup for version {}", version);
        Ok(key_package)
    }

    /// Returns `true` if the caller should acknowledge the transition (send op 23).
    pub fn prepare_transition(&mut self, transition_id: u16, protocol_version: u16) -> bool {
        self.pending_transitions
            .insert(transition_id, protocol_version);
        if transition_id == 0 {
            self.execute_transition(0);
            return false;
        }
        true
    }

    pub fn reset(&mut self) {
        self.protocol_version = 0;
        self.pending_transitions.clear();
        self.external_sender_set = false;
        self.pending_proposals.clear();
        self.was_ready = false;
        // Drop the session entirely — it is invalid after a reset.
        self.session = None;
        info!("DAVE session reset to plaintext/passthrough due to error");
    }

    pub fn execute_transition(&mut self, transition_id: u16) {
        if let Some(next_version) = self.pending_transitions.remove(&transition_id) {
            self.protocol_version = next_version;
            info!(
                "DAVE transition {} executed, protocol version now {}",
                transition_id, next_version
            );
        }
    }

    pub fn prepare_epoch(&mut self, epoch: u64, protocol_version: u16) {
        if epoch == 1 {
            self.protocol_version = protocol_version;
            if let Err(e) = self.setup_session(protocol_version) {
                warn!("DAVE prepare_epoch: setup_session failed: {}", e);
            }
        }
    }

    pub fn process_external_sender(
        &mut self,
        data: &[u8],
        connected_users: &HashSet<crate::common::types::UserId>,
    ) -> AnyResult<Vec<Vec<u8>>> {
        let mut responses = Vec::new();

        if let Some(session) = &mut self.session {
            session.set_external_sender(data).map_err(map_boxed_err)?;
            self.external_sender_set = true;

            if !self.pending_proposals.is_empty() {
                let pending = std::mem::take(&mut self.pending_proposals);
                debug!("DAVE: Processing {} buffered proposals", pending.len());

                for prop_data in pending {
                    if let Ok(Some(res)) =
                        Self::do_process_proposals(session, &prop_data, connected_users)
                    {
                        responses.push(res);
                    }
                }
            }
        }
        Ok(responses)
    }

    pub fn process_welcome(&mut self, data: &[u8]) -> AnyResult<u16> {
        if data.len() < 2 {
            return Err(short_payload_err("DAVE welcome"));
        }
        let transition_id = u16::from_be_bytes([data[0], data[1]]);
        if let Some(session) = &mut self.session {
            session.process_welcome(&data[2..]).map_err(map_boxed_err)?;
            if transition_id != 0 {
                self.pending_transitions
                    .insert(transition_id, self.protocol_version);
            }
            debug!("DAVE welcome processed for transition {}", transition_id);
        }
        Ok(transition_id)
    }

    pub fn process_commit(&mut self, data: &[u8]) -> AnyResult<u16> {
        if data.len() < 2 {
            return Err(short_payload_err("DAVE commit"));
        }
        let transition_id = u16::from_be_bytes([data[0], data[1]]);
        if let Some(session) = &mut self.session {
            session.process_commit(&data[2..]).map_err(map_boxed_err)?;
            if transition_id != 0 {
                self.pending_transitions
                    .insert(transition_id, self.protocol_version);
            }
            debug!("DAVE commit processed for transition {}", transition_id);
        }
        Ok(transition_id)
    }

    pub fn process_proposals(
        &mut self,
        data: &[u8],
        connected_users: &HashSet<crate::common::types::UserId>,
    ) -> AnyResult<Option<Vec<u8>>> {
        if data.is_empty() {
            return Err(short_payload_err("DAVE proposals"));
        }

        if !self.external_sender_set {
            if self.pending_proposals.len() < MAX_PENDING_PROPOSALS {
                debug!(
                    "DAVE: Buffering proposal ({} bytes) — external sender not set yet",
                    data.len()
                );
                self.pending_proposals.push(data.to_vec());
            } else {
                warn!(
                    "DAVE: Proposal buffer full ({} entries), dropping incoming proposal",
                    MAX_PENDING_PROPOSALS
                );
            }
            return Ok(None);
        }

        let session = match &mut self.session {
            Some(s) => s,
            None => return Ok(None),
        };
        Self::do_process_proposals(session, data, connected_users)
    }

    /// Inner implementation that operates directly on a `DaveSession` reference
    /// to avoid borrowing `self` mutably in two places simultaneously.
    fn do_process_proposals(
        session: &mut DaveSession,
        data: &[u8],
        connected_users: &HashSet<crate::common::types::UserId>,
    ) -> AnyResult<Option<Vec<u8>>> {
        let op_type = match data[0] {
            0 => ProposalsOperationType::APPEND,
            1 => ProposalsOperationType::REVOKE,
            raw => {
                return Err(map_boxed_err(format!(
                    "Unknown DAVE proposals op type {}",
                    raw
                )));
            }
        };

        let user_ids: Vec<u64> = connected_users.iter().map(|u| u.0).collect();
        let result = session
            .process_proposals(op_type, &data[1..], Some(&user_ids))
            .map_err(map_boxed_err)?;

        if let Some(cw) = result {
            let mut out = cw.commit;
            if let Some(w) = cw.welcome {
                out.extend_from_slice(&w);
            }
            return Ok(Some(out));
        }
        Ok(None)
    }

    pub fn encrypt_opus(&mut self, packet: &[u8]) -> AnyResult<Vec<u8>> {
        // Pass Discord silence frames through unmodified.
        if packet == SILENCE_FRAME {
            return Ok(packet.to_vec());
        }

        if self.protocol_version == 0 {
            return Ok(packet.to_vec());
        }

        if let Some(session) = &mut self.session {
            let is_ready = session.is_ready();

            if is_ready != self.was_ready {
                if is_ready {
                    info!(
                        "DAVE session (v{}) is now READY — starting encrypted transmission",
                        self.protocol_version
                    );
                } else {
                    warn!(
                        "DAVE session (v{}) LOST readiness — falling back to plaintext",
                        self.protocol_version
                    );
                }
                self.was_ready = is_ready;
            }

            if is_ready {
                return session
                    .encrypt_opus(packet)
                    .map(|c| c.into_owned())
                    .map_err(map_boxed_err);
            }
        }
        Ok(packet.to_vec())
    }
}

/// Creates a short, consistent `AnyError` for truncated payloads.
#[inline]
fn short_payload_err(context: &str) -> AnyError {
    map_boxed_err(format!("Invalid {context} payload: too short"))
}
