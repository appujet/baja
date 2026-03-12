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
    saved_external_sender: Option<Vec<u8>>,
    pending_proposals: Vec<Vec<u8>>,
    pending_handshake: Vec<(Vec<u8>, bool)>,
    was_ready: bool,
    recognized_users: HashSet<UserId>,
    cached_user_ids: Vec<u64>,
}

impl DaveHandler {
    /// Creates a new DaveHandler initialized for the given user and channel.
    ///
    /// The handler starts with no active session, protocol version 0, empty buffers,
    /// and the provided user recognized (cached in `cached_user_ids`).
    ///
    /// # Examples
    ///
    /// ```
    /// let handler = super::DaveHandler::new(UserId(1), ChannelId(2));
    /// assert_eq!(handler.user_id.0, 1);
    /// assert_eq!(handler.channel_id.0, 2);
    /// assert_eq!(handler.protocol_version, 0);
    /// assert_eq!(handler.cached_user_ids, vec![1]);
    /// ```
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
            saved_external_sender: None,
            pending_proposals: Vec::new(),
            pending_handshake: Vec::new(),
            was_ready: false,
            recognized_users,
            cached_user_ids: vec![user_id.0],
        }
    }

    /// Adds the given user IDs to the set of recognized users and refreshes the cached user ID list.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut handler = DaveHandler::new(UserId(1), ChannelId(1));
    /// handler.add_users(&[2, 3, 4]);
    /// assert!(handler.recognized_users.contains(&UserId(2)));
    /// assert!(handler.cached_user_ids.contains(&2));
    /// ```
    pub fn add_users(&mut self, uids: &[u64]) {
        for &uid in uids {
            self.recognized_users.insert(UserId(uid));
        }
        self.update_user_cache();
        debug!("DAVE adding users: {:?}", uids);
    }

    /// Removes the given user identifier from the set of recognized users and updates the cached user ID list.
    ///
    /// If the user was not present, no state is changed.
    ///
    /// # Examples
    ///
    /// ```
    /// use crate::gateway::encryption::{DaveHandler, UserId, ChannelId};
    ///
    /// let mut handler = DaveHandler::new(UserId(1), ChannelId(1));
    /// handler.add_users(&[2]);
    /// handler.remove_user(2);
    /// assert!(!handler.recognized_users.contains(&UserId(2)));
    /// ```
    pub fn remove_user(&mut self, uid: u64) {
        if self.recognized_users.remove(&UserId(uid)) {
            self.update_user_cache();
        }
        debug!("DAVE removing user: {}", uid);
    }

    /// Rebuilds the internal cached list of user IDs from the recognized_users set.
    ///
    /// This updates `cached_user_ids` to reflect the current contents of `recognized_users`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let mut d = DaveHandler::new(UserId(1), ChannelId(1));
    /// d.add_users(&[UserId(2).0]); // adds user 2 to recognized_users
    /// d.update_user_cache();
    /// assert!(d.cached_user_ids.contains(&2));
    /// ```
    fn update_user_cache(&mut self) {
        self.cached_user_ids = self.recognized_users.iter().map(|u| u.0).collect();
    }

    /// Gets the current protocol version used by this handler.
    ///
    /// # Examples
    ///
    /// ```
    /// let handler = DaveHandler::new(UserId(1), ChannelId(1));
    /// assert_eq!(handler.protocol_version(), 0);
    /// ```
    pub fn protocol_version(&self) -> u16 {
        self.protocol_version
    }

    /// Set the active encryption protocol version for this handler.
    ///
    /// A value of `0` disables encryption (handler will treat packets as plaintext).
    ///
    /// # Examples
    ///
    /// ```
    /// let mut h = crate::gateway::encryption::DaveHandler::new(crate::UserId(1), crate::ChannelId(1));
    /// h.set_protocol_version(1);
    /// assert_eq!(h.protocol_version(), 1);
    /// h.set_protocol_version(0);
    /// assert_eq!(h.protocol_version(), 0);
    /// ```
    pub fn set_protocol_version(&mut self, version: u16) {
        self.protocol_version = version;
    }

    /// Initializes or reinitializes the DAVE session for the specified protocol version.
    ///
    /// Creates a new session if none exists or reinitializes the existing session, resets handler state (protocol version, pending buffers, readiness, and external-sender flag), attempts to reapply any previously saved external sender, and returns the session's key package.
    ///
    /// # Parameters
    /// - `version`: protocol version to configure. If `0`, the handler is reset and an empty key package is returned.
    ///
    /// # Returns
    /// The session key package as a `Vec<u8>`. If `version` is `0`, returns an empty vector.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut h = DaveHandler::new(UserId(1), ChannelId(1));
    /// let key_package = h.setup_session(1).unwrap();
    /// assert!(key_package.len() > 0);
    /// ```
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
        self.pending_handshake.clear();
        self.was_ready = false;

        debug!("DAVE session setup (v{})", version);
        let key_package = session.create_key_package().map_err(map_boxed_err)?;

        if let Some(saved) = self.saved_external_sender.clone()
            && let Some(sess) = &mut self.session
        {
            match sess.set_external_sender(&saved) {
                Ok(()) => {
                    self.external_sender_set = true;
                    debug!("DAVE re-applied saved external sender after epoch reset");
                }
                Err(e) => {
                    warn!("DAVE failed to re-apply saved external sender: {e}");
                    self.saved_external_sender = None;
                }
            }
        }

        Ok(key_package)
    }

    pub fn reset(&mut self) {
        self.protocol_version = 0;
        self.pending_transitions.clear();
        self.external_sender_set = false;
        self.saved_external_sender = None;
        self.pending_proposals.clear();
        self.pending_handshake.clear();
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

    /// Configures the external sender from the given payload and processes any buffered proposals and handshake messages.
    ///
    /// This sets the handler's external sender, persists the payload, marks the external sender as configured,
    /// then attempts to process any proposals buffered while the external sender was absent (collecting any response
    /// payloads) and to replay buffered handshake messages. Handshake processing errors are logged but do not stop
    /// proposal response collection.
    ///
    /// # Parameters
    ///
    /// - `data`: Raw external-sender payload bytes to apply.
    ///
    /// # Returns
    ///
    /// `Vec<Vec<u8>>` containing response payloads produced while processing buffered proposals (may be empty).
    ///
    /// # Examples
    ///
    /// ```
    /// let mut handler = DaveHandler::new(UserId(1), ChannelId(1));
    /// let responses = handler.process_external_sender(&[0x01, 0x02]).unwrap();
    /// assert!(responses.is_empty());
    /// ```
    pub fn process_external_sender(&mut self, data: &[u8]) -> AnyResult<Vec<Vec<u8>>> {
        let mut responses = Vec::new();

        if let Some(session) = &mut self.session {
            session.set_external_sender(data).map_err(map_boxed_err)?;
            self.external_sender_set = true;
            self.saved_external_sender = Some(data.to_vec());

            if !self.pending_proposals.is_empty() {
                debug!(
                    "DAVE processing {} buffered proposals",
                    self.pending_proposals.len()
                );
                for prop_data in std::mem::take(&mut self.pending_proposals) {
                    if let Ok(Some(res)) =
                        Self::do_process_proposals(session, &prop_data, &self.cached_user_ids)
                    {
                        responses.push(res);
                    }
                }
            }

            if !self.pending_handshake.is_empty() {
                debug!(
                    "DAVE processing {} buffered handshake messages",
                    self.pending_handshake.len()
                );
                for (handshake_data, is_welcome) in std::mem::take(&mut self.pending_handshake) {
                    if let Err(e) = self.do_process_handshake(&handshake_data, is_welcome) {
                        warn!("DAVE buffered handshake processing failed: {e}");
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

        if !self.external_sender_set {
            if self.pending_handshake.len() < MAX_PENDING_PROPOSALS {
                debug!("DAVE buffering {tag} — external sender not set");
                self.pending_handshake.push((data.to_vec(), is_welcome));
            } else {
                warn!("DAVE handshake buffer full, dropping {tag}");
            }
            return Ok(transition_id);
        }

        self.do_process_handshake(data, is_welcome)?;

        Ok(transition_id)
    }

    fn do_process_handshake(&mut self, data: &[u8], is_welcome: bool) -> AnyResult<()> {
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
            debug!(
                "DAVE {} processed (tid {})",
                if is_welcome { "welcome" } else { "commit" },
                transition_id
            );
        }
        Ok(())
    }

    /// Processes a DAVE proposals payload, returning an optional commit (and possible welcome) response or buffering the payload if the external sender is not yet configured.
    ///
    /// If `data` is empty, an error is returned. If the external sender is not set, the payload is buffered up to `MAX_PENDING_PROPOSALS` and `Ok(None)` is returned. If there is no active session, `Ok(None)` is returned. Otherwise the payload is processed and `Some` response bytes are returned when the session produces a commit (which may include an appended welcome).
    ///
    /// # Examples
    ///
    /// ```
    /// // Minimal usage: with no session or external sender configured this will return `Ok(None)`.
    /// let mut h = DaveHandler::new(UserId(1), ChannelId(1));
    /// let result = h.process_proposals(&[0x00, 0x01]).unwrap();
    /// assert!(result.is_none());
    /// ```
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
        Self::do_process_proposals(session, data, &self.cached_user_ids)
    }

    /// Processes a proposals payload and returns a combined commit (+ optional welcome) payload if produced.
    ///
    /// The first byte of `data` selects the proposals operation (0 = append, 1 = revoke); the remainder is passed
    /// to the session along with `user_ids`.
    ///
    /// # Parameters
    ///
    /// - `session`: mutable reference to an active `DaveSession` used to process the proposals.
    /// - `data`: raw proposals payload where `data[0]` is the operation code and `data[1..]` is the operation body.
    /// - `user_ids`: list of user identifiers to include when processing proposals.
    ///
    /// # Returns
    ///
    /// `Some(Vec<u8>)` containing the commit payload followed by the welcome payload (if produced) when the session
    /// returns a commit; `None` when no output payload is produced.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use crate::gateway::encryption::do_process_proposals;
    /// # use davey::DaveSession;
    /// # let mut session: DaveSession = unimplemented!();
    /// let data = &[0u8, /* proposal body bytes... */][..];
    /// let user_ids = &[1u64, 2u64];
    /// let out = do_process_proposals(&mut session, data, user_ids).unwrap();
    /// match out {
    ///     Some(payload) => println!("Commit (+welcome) payload length: {}", payload.len()),
    ///     None => println!("No payload produced"),
    /// }
    /// ```
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

    /// Encrypts an Opus audio packet with the current DAVE session when encryption is active and the session is ready.
    ///
    /// If the packet equals the configured silence frame or the handler's protocol version is 0, the packet is returned unchanged. When a session exists and reports readiness, the packet is encrypted; otherwise the original packet bytes are returned.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use crate::gateway::encryption::DaveHandler;
    /// # use crate::types::{UserId, ChannelId};
    /// # fn example() -> anyhow::Result<()> {
    /// let mut handler = DaveHandler::new(UserId(1), ChannelId(1));
    /// let raw_packet: Vec<u8> = vec![0u8; 60]; // an Opus packet
    /// let out = handler.encrypt_opus(&raw_packet)?;
    /// // `out` is the encrypted packet when a ready session is active, otherwise `raw_packet`
    /// # Ok(()) }
    /// ```
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

    /// Get the session's voice privacy code as a UTF-8 string, if available.
    ///
    /// # Returns
    ///
    /// `Some(String)` containing the voice privacy code when a session is active and provides one, `None` otherwise.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let code = handler.voice_privacy_code();
    /// if let Some(s) = code {
    ///     println!("voice privacy code: {}", s);
    /// }
    /// ```
    pub fn voice_privacy_code(&self) -> Option<String> {
        self.session
            .as_ref()
            .and_then(|s| s.voice_privacy_code().map(|c| c.to_string()))
    }
}

#[inline]
fn short_payload_err(context: &str) -> AnyError {
    map_boxed_err(format!("Invalid {context} payload: too short"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::types::{ChannelId, UserId};

    #[test]
    fn test_handshake_buffering_logic() {
        let mut handler = DaveHandler::new(UserId(1), ChannelId(1));

        // Buffering should happen if external_sender_set is false
        let welcome_data = vec![0, 42, 1, 2, 3]; // tid 42
        let res = handler.process_welcome(&welcome_data);
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), 42);
        assert_eq!(handler.pending_handshake.len(), 1);

        let commit_data = vec![0, 43, 4, 5, 6]; // tid 43
        let res = handler.process_commit(&commit_data);
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), 43);
        assert_eq!(handler.pending_handshake.len(), 2);

        // setup_session should clear buffers
        handler.setup_session(1).unwrap();
        assert_eq!(handler.pending_handshake.len(), 0);
        assert!(!handler.external_sender_set);

        // Buffering again after setup
        handler.process_welcome(&welcome_data).unwrap();
        assert_eq!(handler.pending_handshake.len(), 1);

        // reset should clear buffers
        handler.reset();
        assert_eq!(handler.pending_handshake.len(), 0);
    }
}
