use std::time::Duration;

use crate::gateway::constants::{BACKOFF_BASE_MS, MAX_RECONNECT_ATTEMPTS};

/// Exponential backoff manager for terminal or recoverable retry loops.
#[derive(Debug, Clone, Default)]
pub struct Backoff {
    attempt: u32,
}

impl Backoff {
    /// Creates a fresh backoff state.
    pub const fn new() -> Self {
        Self { attempt: 0 }
    }

    /// Computes and returns the next delay duration, incrementing the attempt counter.
    pub fn next_delay(&mut self) -> Duration {
        let exponent = self.attempt.min(3);
        let ms = BACKOFF_BASE_MS * 2u64.pow(exponent);
        self.attempt += 1;
        Duration::from_millis(ms)
    }

    /// Returns `true` if the retry limit has been reached.
    #[inline]
    pub const fn is_exhausted(&self) -> bool {
        self.attempt >= MAX_RECONNECT_ATTEMPTS
    }

    /// Resets the counter to zero.
    #[inline]
    pub fn reset(&mut self) {
        self.attempt = 0;
    }

    /// Current attempt count.
    #[inline]
    pub const fn attempt(&self) -> u32 {
        self.attempt
    }
}
