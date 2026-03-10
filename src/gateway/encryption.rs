use std::{
    collections::{HashMap, HashSet},
    num::NonZeroU16,
};

use davey::{DaveSession, ProposalsOperationType};
use tracing::{debug, info, warn};

use crate::{
    common::types::{AnyError, AnyResult, ChannelId, UserId},
    gateway::{
        constants::{DAVE_INITIAL_VERSION, MAX_PENDING_PROPOSALS, SILENCE_FRAME},
        session::types::map_boxed_err,
    },
};

const DAVE_MIN_VERSION: NonZeroU16 = match NonZeroU16::new(DAVE_INITIAL_VERSION) {
    Some(v) => v,
    None => unreachable!(),
};

pub struct DaveHandler {
    session: Option<DaveSession>,
    user_id: UserId,
    channel_id: ChannelId,
    protocol_version: u16,
    pending_transitions: HashMap<u16, u16>,
    external_sender_set: bool,
    pending_proposals: Vec<Vec<u8>>,
    was_ready: bool,
    recognized_users: HashSet<UserId>,
}

impl DaveHandler {
    pub fn new(user_id: UserId, channel_id: ChannelId) -> Self {
        let mut recognized_users = HashSet::new();
        recognized_users.insert(user_id);
        Self {
            session: None,
            user_id,
            channel_id,
            protocol_version: 0,
            pending_transitions: HashMap::new(),
            external_sender_set: false,
            pending_proposals: Vec::new(),
            was_ready: false,
            recognized_users,
        }
    }

    pub fn add_users(&mut self, uids: &[u64]) {
        for &uid in uids {
            self.recognized_users.insert(UserId(uid));
        }
        debug!("DAVE adding users: {:?}", uids);
    }

    pub fn remove_user(&mut self, uid: u64) {
        self.recognized_users.remove(&UserId(uid));
        debug!("DAVE removing user: {}", uid);
    }

    pub fn set_protocol_version(&mut self, version: u16) {
        self.protocol_version = version;
    }

    pub fn setup_session(&mut self, version: u16) -> AnyResult<Vec<u8>> {
        if version == 0 {
            self.reset();
            return Ok(Vec::new());
        }

        let nz_version = NonZeroU16::new(version).unwrap_or(DAVE_MIN_VERSION);

        let session = if let Some(s) = &mut self.session {
            s.reinit(nz_version, self.user_id.0, self.channel_id.0, None)
                .map_err(map_boxed_err)?;
            s
        } else {
            let session = DaveSession::new(nz_version, self.user_id.0, self.channel_id.0, None)
                .map_err(map_boxed_err)?;
            self.session = Some(session);
            self.session.as_mut().unwrap()
        };

        self.protocol_version = version;
        self.external_sender_set = false;
        self.pending_proposals.clear();
        self.was_ready = false;

        debug!("DAVE session setup (v{})", version);
        session.create_key_package().map_err(map_boxed_err)
    }

    pub fn reset(&mut self) {
        self.protocol_version = 0;
        self.pending_transitions.clear();
        self.external_sender_set = false;
        self.pending_proposals.clear();
        self.was_ready = false;
        self.session = None;
        info!("DAVE session reset to plaintext");
    }

    pub fn prepare_transition(&mut self, transition_id: u16, protocol_version: u16) -> bool {
        self.pending_transitions
            .insert(transition_id, protocol_version);

        if transition_id == 0 {
            self.execute_transition(0);
            return false;
        }
        true
    }

    pub fn execute_transition(&mut self, transition_id: u16) {
        if let Some(next_version) = self.pending_transitions.remove(&transition_id) {
            self.protocol_version = next_version;
            info!(
                "DAVE transition {} executed (v{})",
                transition_id, next_version
            );
        }
    }

    pub fn prepare_epoch(&mut self, epoch: u64, protocol_version: u16) -> Option<Vec<u8>> {
        if epoch == 1 {
            match self.setup_session(protocol_version) {
                Ok(kp) => return Some(kp),
                Err(e) => warn!("DAVE prepare_epoch setup failed: {e}"),
            }
        }
        None
    }

    pub fn process_external_sender(&mut self, data: &[u8]) -> AnyResult<Vec<Vec<u8>>> {
        let mut responses = Vec::new();

        if let Some(session) = &mut self.session {
            session.set_external_sender(data).map_err(map_boxed_err)?;
            self.external_sender_set = true;

            if !self.pending_proposals.is_empty() {
                debug!(
                    "DAVE processing {} buffered proposals",
                    self.pending_proposals.len()
                );
                let user_ids: Vec<u64> = self.recognized_users.iter().map(|u| u.0).collect();
                for prop_data in std::mem::take(&mut self.pending_proposals) {
                    if let Ok(Some(res)) =
                        Self::do_process_proposals(session, &prop_data, &user_ids)
                    {
                        responses.push(res);
                    }
                }
            }
        }
        Ok(responses)
    }

    pub fn process_welcome(&mut self, data: &[u8]) -> AnyResult<u16> {
        self.process_handshake_message(data, true)
    }

    pub fn process_commit(&mut self, data: &[u8]) -> AnyResult<u16> {
        self.process_handshake_message(data, false)
    }

    fn process_handshake_message(&mut self, data: &[u8], is_welcome: bool) -> AnyResult<u16> {
        let tag = if is_welcome { "welcome" } else { "commit" };
        if data.len() < 2 {
            return Err(short_payload_err(&format!("DAVE {tag}")));
        }

        let transition_id = u16::from_be_bytes([data[0], data[1]]);
        if let Some(session) = &mut self.session {
            if is_welcome {
                session.process_welcome(&data[2..]).map_err(map_boxed_err)?;
            } else {
                session.process_commit(&data[2..]).map_err(map_boxed_err)?;
            }

            if transition_id != 0 {
                self.pending_transitions
                    .insert(transition_id, self.protocol_version);
            }
            debug!("DAVE {tag} processed (tid {})", transition_id);
        }
        Ok(transition_id)
    }

    pub fn process_proposals(&mut self, data: &[u8]) -> AnyResult<Option<Vec<u8>>> {
        if data.is_empty() {
            return Err(short_payload_err("DAVE proposals"));
        }

        if !self.external_sender_set {
            if self.pending_proposals.len() < MAX_PENDING_PROPOSALS {
                debug!("DAVE buffering proposal — external sender not set");
                self.pending_proposals.push(data.to_vec());
            } else {
                warn!("DAVE proposal buffer full, dropping proposal");
            }
            return Ok(None);
        }

        let session = match &mut self.session {
            Some(s) => s,
            None => return Ok(None),
        };
        let user_ids: Vec<u64> = self.recognized_users.iter().map(|u| u.0).collect();
        Self::do_process_proposals(session, data, &user_ids)
    }

    fn do_process_proposals(
        session: &mut DaveSession,
        data: &[u8],
        user_ids: &[u64],
    ) -> AnyResult<Option<Vec<u8>>> {
        let op_type = match data[0] {
            0 => ProposalsOperationType::APPEND,
            1 => ProposalsOperationType::REVOKE,
            raw => return Err(map_boxed_err(format!("Unknown DAVE proposals op: {raw}"))),
        };

        let result = session
            .process_proposals(op_type, &data[1..], Some(user_ids))
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
        if packet == SILENCE_FRAME || self.protocol_version == 0 {
            return Ok(packet.to_vec());
        }

        if let Some(session) = &mut self.session {
            let is_ready = session.is_ready();

            if is_ready != self.was_ready {
                if is_ready {
                    info!("DAVE session (v{}) is READY", self.protocol_version);
                } else {
                    warn!("DAVE session (v{}) LOST readiness", self.protocol_version);
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

#[inline]
fn short_payload_err(context: &str) -> AnyError {
    map_boxed_err(format!("Invalid {context} payload: too short"))
}
