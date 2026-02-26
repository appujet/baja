//! Central constants for the audio pipeline.
//!
//! All magic numbers in `src/audio/**` live here so they can be tuned in one
//! place and remain consistent across modules.

// ── Sample / PCM ─────────────────────────────────────────────────────────────

/// Output sample rate sent to Discord (Hz).
pub const TARGET_SAMPLE_RATE: u32 = 48_000;

/// Opus/PCM output sample rate as a float, used in filter maths.
pub const SAMPLE_RATE_F64: f64 = 48_000.0;

/// Samples per 20 ms stereo frame at 48 kHz (960 frames × 2 channels).
pub const FRAME_SIZE_SAMPLES: usize = 960 * 2;

/// Stereo channel count used throughout the mixer.
pub const MIXER_CHANNELS: usize = 2;

/// Opus clock rate for position tracking (samples per second, mono).
pub const OPUS_SAMPLE_RATE: u64 = 48_000;

// ── i16 PCM clip boundaries ──────────────────────────────────────────────────

pub const INT16_MAX_F: f32 = 32_767.0;
pub const INT16_MIN_F: f32 = -32_768.0;
pub const INT16_MAX_F64: f64 = 32_768.0; // also used as scale factor
pub const INV_INT16: f64 = 1.0 / INT16_MAX_F64;

// ── Codec ─────────────────────────────────────────────────────────────────────

/// Maximum decoded Opus frame size at 48 kHz: 120 ms → 5 760 samples/channel.
pub const MAX_OPUS_FRAME_SIZE: usize = 5_760;

// ── Buffer pool (byte pool) ───────────────────────────────────────────────────

/// Maximum total bytes held in the byte pool (2 MB — keep it lean).
pub const MAX_POOL_BYTES: usize = 2 * 1_024 * 1_024;

/// Maximum buffers per same-size bucket.
pub const MAX_BUCKET_ENTRIES: usize = 8;

/// Idle duration before the pool is evicted (seconds).
pub const POOL_IDLE_CLEAR_SECS: u64 = 180;

// ── Audio mixer layers ────────────────────────────────────────────────────────

/// Maximum concurrent audio layers in `AudioMixer`.
pub const MAX_LAYERS: usize = 5;

/// Ring-buffer size per layer: 1 MB ≈ 5.4 s of 48 kHz stereo PCM.
pub const LAYER_BUFFER_SIZE: usize = 1_024 * 1_024;

// ── Segmented remote reader ───────────────────────────────────────────────────

/// HTTP fetch chunk size (128 KB) — smaller = faster eviction, less memory spike.
pub const CHUNK_SIZE: usize = 128 * 1_024;

/// Chunks to pre-fetch ahead of the read position (2 = 256 KB look-ahead).
pub const PREFETCH_CHUNKS: usize = 2;

/// Maximum simultaneous in-flight HTTP fetches.
pub const MAX_CONCURRENT_FETCHES: usize = 2;

/// Maximum times a single chunk is retried before the source raises a fatal error.
pub const MAX_FETCH_RETRIES: usize = 5;

/// Milliseconds a fetch worker parks on the condvar when there is no work to do.
pub const WORKER_IDLE_MS: u64 = 50;

/// Milliseconds the reader / worker waits on the condvar per iteration when a
/// chunk is not yet ready.
pub const FETCH_WAIT_MS: u64 = 500;

/// Timeout in seconds for the initial content-length probe request.
pub const PROBE_TIMEOUT_SECS: u64 = 10;

// ── HttpSource ───────────────────────────────────────────────────────────────

/// HTTP prefetch buffer size (8 MB) — how much data can be stored ahead.
pub const HTTP_PREFETCH_BUFFER_SIZE: usize = 8 * 1_024 * 1_024;

/// Smallest buffer capacity used for initial fetching (256 KB).
pub const HTTP_INITIAL_BUF_CAPACITY: usize = 256 * 1_024;

/// Limit for socket-skipping during forward seeks (1 MB).
/// Small forward jumps are faster to skip data on the same connection.
pub const HTTP_SOCKET_SKIP_LIMIT: u64 = 1_000_000;

/// Limit for a single HTTP range fetch in the prefetch loop (5 MB).
/// Prevents excessively large requests and allows for more granular seeking.
pub const HTTP_FETCH_CHUNK_LIMIT: u64 = 5 * 1_024 * 1_024;

// ── Effects ───────────────────────────────────────────────────────────────────

/// π / 2 — used in sinusoidal crossfade curves.
pub const HALF_PI: f32 = std::f32::consts::PI / 2.0;
