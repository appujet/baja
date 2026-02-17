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
}

impl DaveHandler {
    pub fn new(user_id: u64, channel_id: u64) -> Self {
        Self {
            session: None,
            user_id,
            channel_id,
            protocol_version: 0,
            pending_transitions: HashMap::new(),
        }
    }

    pub fn setup_session(&mut self, version: u16) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        self.protocol_version = version;
        let nz_version = NonZeroU16::new(version).unwrap_or(NonZeroU16::new(1).unwrap());

        if let Some(session) = &mut self.session {
            session.reinit(nz_version, self.user_id, self.channel_id, None)?;
        } else {
            self.session = Some(DaveSession::new(
                nz_version,
                self.user_id,
                self.channel_id,
                None,
            )?);
        }

        let key_package = self.session.as_mut().unwrap().create_key_package()?;
        info!("DAVE session setup for version {}", version);
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
    ) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(session) = &mut self.session {
            session.set_external_sender(data)?;
        }
        Ok(())
    }

    pub fn process_welcome(&mut self, data: &[u8]) -> Result<u16, Box<dyn std::error::Error>> {
        if data.len() < 2 {
            return Err("Invalid DAVE welcome payload".into());
        }
        let transition_id = u16::from_be_bytes([data[0], data[1]]);
        if let Some(session) = &mut self.session {
            session.process_welcome(&data[2..])?;
            if transition_id != 0 {
                self.pending_transitions
                    .insert(transition_id, self.protocol_version);
            }
            info!("DAVE welcome processed for transition {}", transition_id);
        }
        Ok(transition_id)
    }

    pub fn process_commit(&mut self, data: &[u8]) -> Result<u16, Box<dyn std::error::Error>> {
        if data.len() < 2 {
            return Err("Invalid DAVE commit payload".into());
        }
        let transition_id = u16::from_be_bytes([data[0], data[1]]);
        if let Some(session) = &mut self.session {
            session.process_commit(&data[2..])?;
            if transition_id != 0 {
                self.pending_transitions
                    .insert(transition_id, self.protocol_version);
            }
            info!("DAVE commit processed for transition {}", transition_id);
        }
        Ok(transition_id)
    }

    pub fn process_proposals(
        &mut self,
        data: &[u8],
        connected_users: &HashSet<u64>,
    ) -> Result<Option<Vec<u8>>, Box<dyn std::error::Error>> {
        if data.is_empty() {
            return Err("Empty DAVE proposals payload".into());
        }
        let op_type_raw = data[0];
        let op_type = match op_type_raw {
            0 => ProposalsOperationType::APPEND,
            1 => ProposalsOperationType::REVOKE,
            _ => return Err(format!("Unknown DAVE proposals op type {}", op_type_raw).into()),
        };

        if let Some(session) = &mut self.session {
            let user_ids: Vec<u64> = connected_users.iter().cloned().collect();
            let result = session.process_proposals(op_type, &data[1..], Some(&user_ids))?;
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

    pub fn encrypt_opus(&mut self, packet: &[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        if self.protocol_version == 0 {
            return Ok(packet.to_vec());
        }
        if let Some(session) = &mut self.session {
            if session.is_ready() {
                let encrypted = session.encrypt_opus(packet)?;
                return Ok(encrypted.into_owned());
            }
        }
        Ok(packet.to_vec())
    }
}
