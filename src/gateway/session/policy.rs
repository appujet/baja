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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_policy_new() {
        let policy = FailurePolicy::new(5);
        assert!(policy.is_retryable(1000, 0));
    }

    #[test]
    fn test_is_retryable_normal_codes() {
        let policy = FailurePolicy::new(3);

        // Within retry limit
        assert!(policy.is_retryable(1000, 0)); // NORMAL
        assert!(policy.is_retryable(1001, 1)); // GOING_AWAY
        assert!(policy.is_retryable(1006, 2)); // ABNORMAL_CLOSURE

        // At retry limit
        assert!(!policy.is_retryable(1000, 3));
        assert!(!policy.is_retryable(1006, 5));
    }

    #[test]
    fn test_is_retryable_discord_codes() {
        let policy = FailurePolicy::new(3);

        assert!(policy.is_retryable(4000, 0)); // INTERNAL_ERROR
        assert!(policy.is_retryable(4001, 1)); // UNKNOWN_OPCODE
        assert!(policy.is_retryable(4002, 0)); // FAILED_TO_DECODE_PAYLOAD
        assert!(policy.is_retryable(4003, 1)); // NOT_AUTHENTICATED
        assert!(policy.is_retryable(4005, 2)); // ALREADY_AUTHENTICATED
        assert!(policy.is_retryable(4006, 0)); // SESSION_NO_LONGER_VALID
        assert!(policy.is_retryable(4009, 1)); // SESSION_TIMEOUT
        assert!(policy.is_retryable(4012, 0)); // UNKNOWN_PROTOCOL
        assert!(policy.is_retryable(4015, 1)); // VOICE_SERVER_CRASHED
        assert!(policy.is_retryable(4016, 0)); // UNKNOWN_ENCRYPTION_MODE
        assert!(policy.is_retryable(4020, 1)); // BAD_REQUEST
        assert!(policy.is_retryable(4900, 2)); // RECONNECT
    }

    #[test]
    fn test_is_not_retryable_codes() {
        let policy = FailurePolicy::new(3);

        // Non-retryable codes
        assert!(!policy.is_retryable(4004, 0)); // Not in retryable list
        assert!(!policy.is_retryable(4011, 1));
        assert!(!policy.is_retryable(4999, 0)); // Unknown code
        assert!(!policy.is_retryable(1002, 1)); // Not in retryable list
    }

    #[test]
    fn test_classify_shutdown_codes() {
        let policy = FailurePolicy::new(3);

        assert_eq!(policy.classify(4004), SessionOutcome::Shutdown);
        assert_eq!(policy.classify(4011), SessionOutcome::Shutdown);
        assert_eq!(policy.classify(4021), SessionOutcome::Shutdown);
        assert_eq!(policy.classify(4022), SessionOutcome::Shutdown);
    }

    #[test]
    fn test_classify_identify_codes() {
        let policy = FailurePolicy::new(3);

        assert_eq!(policy.classify(4006), SessionOutcome::Identify);
        assert_eq!(policy.classify(4009), SessionOutcome::Identify);
        assert_eq!(policy.classify(4014), SessionOutcome::Identify);
    }

    #[test]
    fn test_classify_reconnect_codes() {
        let policy = FailurePolicy::new(3);

        // All other codes should result in Reconnect
        assert_eq!(policy.classify(1000), SessionOutcome::Reconnect);
        assert_eq!(policy.classify(1001), SessionOutcome::Reconnect);
        assert_eq!(policy.classify(4000), SessionOutcome::Reconnect);
        assert_eq!(policy.classify(4001), SessionOutcome::Reconnect);
        assert_eq!(policy.classify(4999), SessionOutcome::Reconnect);
        assert_eq!(policy.classify(5000), SessionOutcome::Reconnect);
    }

    #[test]
    fn test_retry_boundary() {
        let policy = FailurePolicy::new(3);

        // Just before limit
        assert!(policy.is_retryable(1000, 2));

        // At limit
        assert!(!policy.is_retryable(1000, 3));

        // Beyond limit
        assert!(!policy.is_retryable(1000, 4));
        assert!(!policy.is_retryable(1000, 100));
    }

    #[test]
    fn test_zero_retries() {
        let policy = FailurePolicy::new(0);

        // No retries allowed
        assert!(!policy.is_retryable(1000, 0));
        assert!(!policy.is_retryable(4000, 0));
    }
}
