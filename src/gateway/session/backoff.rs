use std::time::Duration;

use crate::gateway::constants::{BACKOFF_BASE_MS, MAX_RECONNECT_ATTEMPTS};

/// Exponential backoff manager for terminal or recoverable retry loops.
#[derive(Debug, Clone, Default)]
pub struct Backoff {
    attempt: u32,
}

impl Backoff {
    /// Creates a fresh Backoff with the attempt counter initialized to 0.
    ///
    /// # Examples
    ///
    /// ```
    /// let b = Backoff::new();
    /// assert_eq!(b.attempt(), 0);
    /// ```
    pub const fn new() -> Self {
        Self { attempt: 0 }
    }

    /// Computes the next backoff delay and advances the internal attempt counter.
    ///
    /// The returned `Duration` is the delay to wait before the next retry. Calling this
    /// method mutates the receiver by incrementing its attempt count.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    ///
    /// let mut b = Backoff::new();
    /// let d1 = b.next_delay();
    /// assert_eq!(b.attempt(), 1);
    /// let d2 = b.next_delay();
    /// assert_eq!(b.attempt(), 2);
    /// assert!(d2 >= d1);
    /// ```
    pub fn next_delay(&mut self) -> Duration {
        let exponent = self.attempt.min(3);
        let ms = BACKOFF_BASE_MS * 2u64.pow(exponent);
        self.attempt += 1;
        Duration::from_millis(ms)
    }

    /// Reports whether the backoff has reached the configured maximum number of attempts.
    ///
    /// # Returns
    ///
    /// `true` if the current attempt count is greater than or equal to `MAX_RECONNECT_ATTEMPTS`, `false` otherwise.
    ///
    /// # Examples
    ///
    /// ```
    /// let b = Backoff::new();
    /// assert!(!b.is_exhausted());
    /// ```
    #[inline]
    pub const fn is_exhausted(&self) -> bool {
        self.attempt >= MAX_RECONNECT_ATTEMPTS
    }

    /// Resets the attempt counter to zero.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut b = Backoff::new();
    /// b.next_delay();
    /// b.reset();
    /// assert_eq!(b.attempt(), 0);
    /// ```
    #[inline]
    pub fn reset(&mut self) {
        self.attempt = 0;
    }

    /// Gets the current retry attempt count.
    ///
    /// # Returns
    ///
    /// The current attempt count.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut backoff = Backoff::new();
    /// assert_eq!(backoff.attempt(), 0);
    /// backoff.next_delay();
    /// assert_eq!(backoff.attempt(), 1);
    /// ```
    #[inline]
    pub const fn attempt(&self) -> u32 {
        self.attempt
    }
}
