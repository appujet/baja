/// Discord Voice Gateway version to use in the WebSocket URL.
pub const VOICE_GATEWAY_VERSION: u8 = 8;

/// Default audio sample rate (48 kHz) used by Discord voice.
pub const DEFAULT_SAMPLE_RATE: u32 = 48_000;

/// Maximum reconnect attempts before giving up on a voice session.
pub const MAX_RECONNECT_ATTEMPTS: u32 = 5;

/// Base delay (ms) for the exponential backoff on reconnect.
pub const BACKOFF_BASE_MS: u64 = 1_000;

/// Fixed delay (ms) before a fresh Identify after a session-invalid close.
pub const RECONNECT_DELAY_FRESH_MS: u64 = 500;

/// Timeout (ms) allowed for the WS write task to shut down gracefully.
pub const WRITE_TASK_SHUTDOWN_MS: u64 = 500;
