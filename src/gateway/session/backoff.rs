use std::time::Duration;

use super::super::constants::{BACKOFF_BASE_MS, MAX_RECONNECT_ATTEMPTS};

pub(super) struct Backoff {
    attempt: u32,
}

impl Backoff {
    pub(super) fn new() -> Self {
        Self { attempt: 0 }
    }

    /// Returns the next back-off delay and increments the attempt counter.
    pub(super) fn next(&mut self) -> Duration {
        self.attempt += 1;
        let delay = BACKOFF_BASE_MS * 2u64.pow((self.attempt - 1).min(3));
        Duration::from_millis(delay)
    }

    pub(super) fn is_exhausted(&self) -> bool {
        self.attempt >= MAX_RECONNECT_ATTEMPTS
    }
}
