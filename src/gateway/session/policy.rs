use super::types::SessionOutcome;

/// Defines the strategy for handling session failures and closure codes.
pub struct FailurePolicy {
    max_retries: u32,
}

impl FailurePolicy {
    pub const fn new(max_retries: u32) -> Self {
        Self { max_retries }
    }

    /// Determines if a WebSocket close code is retryable without notifying the client.
    ///
    /// these codes trigger an internal reconnection attempt
    /// and should be suppressed from external event emissions during the initial retry phase.
    pub const fn is_retryable(&self, code: u16, attempt: u32) -> bool {
        if attempt >= self.max_retries {
            return false;
        }

        matches!(
            code,
            1000 | // NORMAL (User requested suppression)
            1001 | // GOING_AWAY
            1006 | // ABNORMAL_CLOSURE
            4000 | // INTERNAL_ERROR
            4001 | // UNKNOWN_OPCODE
            4002 | // FAILED_TO_DECODE_PAYLOAD
            4003 | // NOT_AUTHENTICATED
            4005 | // ALREADY_AUTHENTICATED
            4006 | // SESSION_NO_LONGER_VALID
            4009 | // SESSION_TIMEOUT
            4012 | // UNKNOWN_PROTOCOL
            4015 | // VOICE_SERVER_CRASHED
            4016 | // UNKNOWN_ENCRYPTION_MODE
            4020 | // BAD_REQUEST
            4900 // RECONNECT
        )
    }

    /// Maps a Discord closure code to a high-level session outcome.
    pub fn classify(&self, code: u16) -> SessionOutcome {
        match code {
            4004 | 4011 | 4021 | 4022 => SessionOutcome::Shutdown,
            4006 | 4009 | 4014 => SessionOutcome::Identify,
            _ => SessionOutcome::Reconnect,
        }
    }
}
