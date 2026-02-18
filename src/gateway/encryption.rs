use davey::{DaveSession, ProposalsOperationType};
use std::collections::{HashMap, HashSet};
use std::num::NonZeroU16;
use tracing::info;

pub struct DaveHandler {
    session: Option<DaveSession>,
    user_id: u64,
    channel_id: u64,
    protocol_version: u16,
    pending_transitions: HashMap<u16, u16>,
    external_sender_set: bool,
    pending_proposals: Vec<Vec<u8>>,
    was_ready: bool,
}

fn map_boxed_err<E: std::fmt::Display>(e: E) -> Box<dyn std::error::Error + Send + Sync> {
    Box::new(std::io::Error::new(
        std::io::ErrorKind::Other,
        e.to_string(),
    ))
}

impl DaveHandler {
    pub fn new(user_id: u64, channel_id: u64) -> Self {
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

    pub fn setup_session(
        &mut self,
        version: u16,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        self.protocol_version = version;
        let nz_version = NonZeroU16::new(version).unwrap_or(NonZeroU16::new(1).unwrap());

        if let Some(session) = &mut self.session {
            session
                .reinit(nz_version, self.user_id, self.channel_id, None)
                .map_err(map_boxed_err)?;
        } else {
            self.session = Some(
                DaveSession::new(nz_version, self.user_id, self.channel_id, None)
                    .map_err(map_boxed_err)?,
            );
        }

        // Reset state on session (re)init
        self.external_sender_set = false;
        self.pending_proposals.clear();
        self.was_ready = false;

        let key_package = self
            .session
            .as_mut()
            .unwrap()
            .create_key_package()
            .map_err(map_boxed_err)?;
        tracing::debug!("DAVE session setup for version {}", version);
        Ok(key_package)
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
                "DAVE transition {} executed, protocol version now {}",
                transition_id, next_version
            );
        }
    }

    pub fn prepare_epoch(&mut self, epoch: u64, protocol_version: u16) {
        if epoch == 1 {
            self.protocol_version = protocol_version;
            let _ = self.setup_session(protocol_version);
        }
    }

    pub fn process_external_sender(
        &mut self,
        data: &[u8],
        connected_users: &HashSet<u64>,
    ) -> Result<Vec<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>> {
        let mut responses = Vec::new();

        if let Some(session) = &mut self.session {
            session.set_external_sender(data).map_err(map_boxed_err)?;
            self.external_sender_set = true;

            if !self.pending_proposals.is_empty() {
                let pending = std::mem::take(&mut self.pending_proposals);
                tracing::debug!("DAVE: Processing {} buffered proposals", pending.len());

                for prop_data in pending {
                    if let Ok(Some(res)) = self.process_proposals(&prop_data, connected_users) {
                        responses.push(res);
                    }
                }
            }
        }
        Ok(responses)
    }

    pub fn process_welcome(
        &mut self,
        data: &[u8],
    ) -> Result<u16, Box<dyn std::error::Error + Send + Sync>> {
        if data.len() < 2 {
            return Err(map_boxed_err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Invalid DAVE welcome payload",
            )));
        }
        let transition_id = u16::from_be_bytes([data[0], data[1]]);
        if let Some(session) = &mut self.session {
            session.process_welcome(&data[2..]).map_err(map_boxed_err)?;
            if transition_id != 0 {
                self.pending_transitions
                    .insert(transition_id, self.protocol_version);
            }
            tracing::debug!("DAVE welcome processed for transition {}", transition_id);
        }
        Ok(transition_id)
    }

    pub fn process_commit(
        &mut self,
        data: &[u8],
    ) -> Result<u16, Box<dyn std::error::Error + Send + Sync>> {
        if data.len() < 2 {
            return Err(map_boxed_err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Invalid DAVE commit payload",
            )));
        }
        let transition_id = u16::from_be_bytes([data[0], data[1]]);
        if let Some(session) = &mut self.session {
            session.process_commit(&data[2..]).map_err(map_boxed_err)?;
            if transition_id != 0 {
                self.pending_transitions
                    .insert(transition_id, self.protocol_version);
            }
            tracing::debug!("DAVE commit processed for transition {}", transition_id);
        }
        Ok(transition_id)
    }

    pub fn process_proposals(
        &mut self,
        data: &[u8],
        connected_users: &HashSet<u64>,
    ) -> Result<Option<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>> {
        if data.is_empty() {
            return Err(map_boxed_err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Empty DAVE proposals payload",
            )));
        }

        if !self.external_sender_set {
            tracing::info!(
                "DAVE: Buffering proposal ({} bytes) - external sender not set yet",
                data.len()
            );
            self.pending_proposals.push(data.to_vec());
            return Ok(None);
        }

        let op_type_raw = data[0];
        let op_type = match op_type_raw {
            0 => ProposalsOperationType::APPEND,
            1 => ProposalsOperationType::REVOKE,
            _ => {
                return Err(map_boxed_err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("Unknown DAVE proposals op type {}", op_type_raw),
                )))
            }
        };

        if let Some(session) = &mut self.session {
            let user_ids: Vec<u64> = connected_users.iter().cloned().collect();
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
        }
        Ok(None)
    }

    fn has_pending_downgrade(&self) -> bool {
        self.pending_transitions.values().any(|&v| v == 0)
    }

    pub fn encrypt_opus(
        &mut self,
        packet: &[u8],
    ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        // Discord special 3-byte silence frame: [0xf8, 0xff, 0xfe]
        if packet.len() == 3 && packet[0] == 0xf8 && packet[1] == 0xff && packet[2] == 0xfe {
            return Ok(packet.to_vec());
        }

        if self.protocol_version == 0 || self.has_pending_downgrade() {
            return Ok(packet.to_vec());
        }

        if let Some(session) = &mut self.session {
            let is_ready = session.is_ready();

            if is_ready != self.was_ready {
                if is_ready {
                    tracing::info!(
                        "DAVE session (v{}) is now READY. Starting encrypted transmission.",
                        self.protocol_version
                    );
                } else {
                    tracing::warn!(
                        "DAVE session (v{}) LOST readiness. Falling back to plaintext.",
                        self.protocol_version
                    );
                }
                self.was_ready = is_ready;
            }

            if is_ready {
                let encrypted = session.encrypt_opus(packet).map_err(map_boxed_err)?;
                return Ok(encrypted.into_owned());
            }
        }
        Ok(packet.to_vec())
    }
}
