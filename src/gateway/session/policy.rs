use super::types::SessionOutcome;

/// Defines the strategy for handling session failures and closure codes.
pub struct FailurePolicy {
    max_retries: u32,
}

impl FailurePolicy {
    /// Creates a FailurePolicy configured with the given maximum number of retry attempts.
    ///
    /// `max_retries` is the maximum number of internal retry attempts the policy will permit
    /// before treating subsequent failures as non-retryable.
    ///
    /// # Examples
    ///
    /// ```
    /// let policy = FailurePolicy::new(3);
    /// ```
    pub const fn new(max_retries: u32) -> Self {
        Self { max_retries }
    }

    /// Decides whether a WebSocket close code should be retried internally without emitting an external event.
    ///
    /// Returns `true` if the provided `attempt` is less than the policy's `max_retries` and `code` is one of the internal
    /// retryable close codes; `false` otherwise.
    ///
    /// # Examples
    ///
    /// ```
    /// let policy = FailurePolicy::new(3);
    /// // Retryable code within attempts
    /// assert!(policy.is_retryable(1006, 0));
    /// // Non-retryable because attempts exhausted
    /// assert!(!policy.is_retryable(1006, 3));
    /// // Non-retryable code
    /// assert!(!policy.is_retryable(4004, 0));
    /// ```
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

    /// Classifies a Discord WebSocket close code into a high-level session outcome.
    ///
    /// Maps specific close codes to `SessionOutcome` variants:
    /// - `4004`, `4011`, `4021`, `4022` => `SessionOutcome::Shutdown`
    /// - `4006`, `4009`, `4014` => `SessionOutcome::Identify`
    /// - all other codes => `SessionOutcome::Reconnect`
    ///
    /// # Examples
    ///
    /// ```
    /// let policy = FailurePolicy::new(3);
    /// assert_eq!(policy.classify(4004), SessionOutcome::Shutdown);
    /// assert_eq!(policy.classify(4009), SessionOutcome::Identify);
    /// assert_eq!(policy.classify(1000), SessionOutcome::Reconnect);
    /// ```
    pub fn classify(&self, code: u16) -> SessionOutcome {
        match code {
            4004 | 4011 | 4021 | 4022 => SessionOutcome::Shutdown,
            4006 | 4009 | 4014 => SessionOutcome::Identify,
            _ => SessionOutcome::Reconnect,
        }
    }
}
