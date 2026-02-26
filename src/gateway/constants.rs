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

/// Initial DAVE protocol version sent on session setup.
pub const DAVE_INITIAL_VERSION: u16 = 1;

/// Maximum number of proposals buffered before the external sender is set.
/// Prevents unbounded memory growth if the server delays the sender packet.
pub const MAX_PENDING_PROPOSALS: usize = 64;

/// Discord 3-byte Opus silence frame (sent to signal end-of-speech).
pub const SILENCE_FRAME: [u8; 3] = [0xf8, 0xff, 0xfe];

/// RTP version + padding/extension/CC flags byte (V=2, no P/X/CC).
pub const RTP_VERSION_BYTE: u8 = 0x80;

/// RTP payload type for Opus audio as used by Discord.
pub const RTP_OPUS_PAYLOAD_TYPE: u8 = 0x78;

/// Number of PCM samples per 20 ms Opus frame at 48 kHz.
pub const RTP_TIMESTAMP_STEP: u32 = 960;

/// Pre-allocated UDP send-buffer capacity (≈ Ethernet MTU).
pub const UDP_PACKET_BUF_CAPACITY: usize = 1500;

/// Default voice encryption mode string negotiated with Discord.
pub const DEFAULT_VOICE_MODE: &str = "xsalsa20_poly1305";

/// WebSocket opcode for heartbeat.
pub const OP_HEARTBEAT: u8 = 3;

/// Size of the IP discovery packet sent to the Discord voice UDP server.
pub const DISCOVERY_PACKET_SIZE: usize = 74;

/// Standard frame duration for Discord audio packets (20ms).
pub const FRAME_DURATION_MS: u64 = 20;

/// Timeout in seconds to wait for an IP discovery response.
pub const IP_DISCOVERY_TIMEOUT_SECS: u64 = 2;

/// Maximum number of silent frames to send before stopping transmission.
pub const MAX_SILENCE_FRAMES: u32 = 5;

/// Maximum size in bytes of an encoded Opus frame.
pub const MAX_OPUS_FRAME_SIZE: usize = 4000;

/// Number of PCM samples per frame (960 at 48kHz for 20ms).
pub const PCM_FRAME_SAMPLES: usize = 960;
