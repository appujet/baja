// --- Versions & Defaults ---
pub const VOICE_GATEWAY_VERSION: u8 = 8;
pub const DEFAULT_SAMPLE_RATE: u32 = 48_000;
pub const DAVE_INITIAL_VERSION: u16 = 1;
pub const DEFAULT_VOICE_MODE: &str = "xsalsa20_poly1305";

// --- Connection & Reconnect ---
pub const MAX_RECONNECT_ATTEMPTS: u32 = 5;
pub const BACKOFF_BASE_MS: u64 = 1_000;
pub const RECONNECT_DELAY_FRESH_MS: u64 = 2000;

pub const UDP_KEEPALIVE_GAP_MS: u64 = 5000;
pub const WRITE_TASK_SHUTDOWN_MS: u64 = 500;

// --- Audio & RTP ---
pub const RTP_VERSION_BYTE: u8 = 0x80;
pub const RTP_OPUS_PAYLOAD_TYPE: u8 = 0x78;
pub const RTP_TIMESTAMP_STEP: u32 = 960;
pub const FRAME_DURATION_MS: u64 = 20;

pub const PCM_FRAME_SAMPLES: usize = 960;
pub const MAX_OPUS_FRAME_SIZE: usize = 4000;
pub const SILENCE_FRAME: [u8; 3] = [0xf8, 0xff, 0xfe];
pub const MAX_SILENCE_FRAMES: u32 = 5;

// --- UDP & Discovery ---
pub const UDP_PACKET_BUF_CAPACITY: usize = 1500;
pub const DISCOVERY_PACKET_SIZE: usize = 74;
pub const IP_DISCOVERY_TIMEOUT_SECS: u64 = 2;

// --- Protocol Specifics ---
pub const OP_HEARTBEAT: u8 = 3;
pub const MAX_PENDING_PROPOSALS: usize = 64;
