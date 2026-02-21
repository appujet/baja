# Code Improvement Suggestions

A prioritized list of improvements for the **Rustalink** codebase, organized by category.

---

## 🔴 High Priority (Correctness / Safety)

### 1. Remove `blocking` feature from `reqwest` in async code
**File:** `Cargo.toml`

The `reqwest` dependency includes the `blocking` feature alongside an async Tokio runtime. Calling `reqwest::blocking` from inside a Tokio async thread panics at runtime because blocking clients cannot be used inside async executors directly. Either:
- Remove `blocking` entirely and replace all usages with `tokio::task::spawn_blocking`.
- Or use `reqwest::Client` (async) everywhere and drop `blocking::Client`.

```toml
# Before
reqwest = { version = "0.12", features = ["blocking", "json", "rustls-tls", "stream", "cookies"] }

# After
reqwest = { version = "0.12", features = ["json", "rustls-tls", "stream", "cookies"] }
```

---

### 2. `MirroredTrack`: Replace manual thread + new Tokio runtime with `spawn_blocking`
**File:** `src/sources/manager.rs` (lines 267–304)

`MirroredTrack::start_decoding` spawns a raw OS thread then immediately creates a **new** `tokio::runtime` inside it. This is wasteful — a new runtime per track — and can exhaust system resources under load. Use `tokio::task::spawn_blocking` from the existing runtime instead, or restructure the interface so `start_decoding` is async.

```rust
// Instead of:
std::thread::spawn(move || {
    let runtime = tokio::runtime::Builder::new_current_thread()...build()...;
    runtime.block_on(async { ... });
});

// Consider:
tokio::spawn(async move {
    // use .await directly
});
```

---

### 3. `CircularFileWriter::write` holds a `Mutex` guard while doing I/O
**File:** `src/common/logger.rs` (lines 176–199)

The `Mutex` guard (`state`) is acquired before the expensive file I/O (`self.prune()`). This blocks every other thread trying to log. Restructure so the mutex only protects the counter, and the pruning happens outside the lock.

```rust
// After updating counter, drop the guard before pruning:
let should_prune = {
    let mut state = self.state.lock().unwrap();
    state.lines_since_prune += new_lines;
    if state.lines_since_prune >= prune_threshold {
        state.lines_since_prune = 0;
        true
    } else {
        false
    }
};
if should_prune {
    let _ = self.prune();
}
```

---

### 4. `.unwrap()` on `Mutex` lock in production paths
**Files:** `src/common/logger.rs`, `src/sources/manager.rs`, multiple source files

`Mutex::lock().unwrap()` will panic if the mutex is poisoned (i.e., a thread panicked while holding it). Prefer `.unwrap_or_else(|e| e.into_inner())` for non-critical state, or propagate the error properly.

---

### 5. `unwrap()` on `write_u8` / `write_u16` in track encode
**File:** `src/api/tracks.rs` (lines 42–58)

All `buf.write_u8/u16/u64()` calls use `.unwrap()`. Since `Vec<u8>` writing is infallible, these will never panic — but they create noise and make reviews harder. Use `let _ = buf.write_u8(...);` or add a comment explaining why it's infallible.

---

## 🟡 Medium Priority (Design / Maintainability)

### 6. `HttpClient` is a zero-sized struct used only as a namespace
**File:** `src/common/http.rs`

`HttpClient` has no fields and all methods are `fn`, not `self`. This is an anti-pattern in Rust. Prefer a module-level `pub fn` or a static/const for the user agent.

```rust
// Instead of HttpClient::new() and HttpClient::default_user_agent()
pub const DEFAULT_USER_AGENT: &str = "Mozilla/5.0 ...";

pub fn new_http_client() -> Result<Client, Error> { ... }
pub fn new_blocking_http_client() -> Result<blocking::Client, Error> { ... }
```

---

### 7. `SourceManager::new` uses `expect()` for non-fatal initialization
**File:** `src/sources/manager.rs` (line 44)

```rust
DeezerSource::new(...).expect("Failed to create Deezer source")
```

A single source failing should not crash the whole server. Return `Result<Self, ...>` from `SourceManager::new` and propagate errors to `main`, or log the error and skip the source gracefully.

---

### 8. Duplicate `VoiceState` / `VoiceConnectionState` types
**File:** `src/player/state.rs` (lines 37–54)

`VoiceState` (for serialization) and `VoiceConnectionState` (internal) share nearly identical fields. Use `VoiceState` everywhere (or make a `From` impl) to avoid keeping two structs in sync.

---

### 9. `GuildId` / `SessionId` are type aliases for `String` — not strongly typed
**File:** `src/common/types.rs` (lines 18–20)

Type aliases like `type GuildId = String` don't prevent misuse — you can accidentally pass a `SessionId` where a `GuildId` is expected. Consider newtype wrappers:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GuildId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionId(pub String);
```

---

### 10. `configs/sources.rs` — Excessive default functions pattern
**File:** `src/configs/sources.rs`

Each config field that has a default spawns its own standalone `fn default_xxx() -> usize { N }`. This is repetitive boilerplate. Use a constant or inline the value directly in the `Default` impl:

```rust
impl Default for SpotifyConfig {
    fn default() -> Self {
        Self {
            playlist_load_limit: 6,
            album_load_limit: 6,
            search_limit: 10,
            ...
        }
    }
}
```

---

### 11. Missing blank line between `SourceManager::load` and `load_search` methods
**File:** `src/sources/manager.rs` (line 116)

Minor style: `load_search` follows `load` without a blank line separator, making the boundary hard to spot on first read.

---

### 12. Banner in `main.rs` uses raw string with embedded ANSI codes
**File:** `src/main.rs` (lines 18–28)

The ASCII art banner embeds ANSI escape codes directly in the source string. Extract them into named constants or use a crate like `colored` / `owo-colors` for maintainability.

---

### 13. Log message says "Lavalink Server" instead of project name
**Files:** `src/main.rs` (lines 30, 61)

```rust
info!("Lavalink Server starting...");
info!("Lavalink Server listening on {}", address);
```

The project is **Rustalink**, not Lavalink. Update these log messages to reflect the actual product name for clearer logs.

---

## 🟢 Low Priority (Performance / Polish)

### 14. `SourceManager::source_names` allocates a `Vec<String>` on every call
**File:** `src/sources/manager.rs` (lines 235–237)

`source_names()` is presumably a debugging helper. If ever called in a hot path, consider returning `impl Iterator<Item = &str>` instead of allocating a new `Vec`.

---

### 15. `strip_ansi_escapes` is a naive implementation
**File:** `src/common/logger.rs` (lines 46–61)

The manual ANSI stripper only handles sequences ending with an ASCII letter. It won't correctly handle `ESC[38;2;r;g;bm` (RGB color) or other multi-part sequences. Use the `strip-ansi-escapes` crate instead:

```toml
strip-ansi-escapes = "0.2"
```

---

### 16. `AudioProcessor` target sample rate is hardcoded to `48000`
**File:** `src/audio/processor.rs` (line 81)

```rust
let target_rate = 48000;
```

This should come from configuration or from the Discord voice connection negotiation. Hardcoding means resampling to 48kHz even when it's unnecessary.

---

### 17. `DecoderCommand::Seek` returns the command redundantly
**File:** `src/audio/processor.rs` (lines 180–193)

`check_commands` returns `Some(DecoderCommand::Seek(ms))` but the caller in `run()` only checks `if matches!(cmd, DecoderCommand::Stop)`. The seek command return value is never used. Consider returning a `bool` (should_stop) instead, or a dedicated enum.

---

### 18. `PlayerUpdate` `end_time` field logic is unclear
**File:** `src/player/state.rs` (line 69)

```rust
pub end_time: Option<Option<u64>>,
```

`Option<Option<u64>>` models three states: "not provided", "explicit null (clear)", and "some value". Consider a dedicated enum for clarity:

```rust
pub enum EndTime {
    Unchanged,
    Clear,
    Set(u64),
}
```

---

### 19. `Session` uses `tokio::sync::Mutex` for `sender` but `std::sync::atomic` for flags
**File:** `src/transport/websocket_server.rs` (line 303–307)

This is correct but could use a code comment explaining why `tokio::sync::Mutex` is used for `sender` (because it's held across `.await` points). Currently it mixes sync and async primitives silently.

---

### 20. No integration or unit tests anywhere in the project
**File:** entire `src/` tree

There are no `#[test]` or `#[cfg(test)]` modules found in the codebase. At minimum, add:
- Unit tests for `Track::encode` / `Track::decode` round-trip correctness.
- Unit tests for `AudioKind::from_ext` / `as_ext`.
- Unit tests for `Filters::merge_from`.
- Integration tests for the HTTP REST endpoints using `axum::test`.

---

### 21. `rand = "0.8.5"` is pinned to an old version
**File:** `Cargo.toml` (line 18)

`rand 0.9` was released and has API improvements. Pin to a range (`^0.8`) rather than an exact patch version to receive future bug/security fixes automatically within the same semver range:

```toml
rand = "^0.8"
```

---

### 22. `config.toml` and `config.default.toml` may drift out of sync
**Files:** `config.toml`, `config.default.toml`

Having two config files that must mirror each other is error-prone. Consider using only `config.default.toml` as the canonical source of truth and having `config.toml` be a user overrides file parsed as a patch on top of the defaults — or add a CI check to diff them.

---

## 📝 Documentation

### 23. Public API types lack `#[doc]` comments
**Files:** `src/api/tracks.rs`, `src/api/events.rs`, `src/api/opcodes.rs`

Many public-facing types and trait methods have no rustdoc comments. Add `///` comments to all public types, especially `SourcePlugin`, `PlayableTrack`, and `LoadResult` variants, to make the plugin API self-documenting.

---

### 24. `SourcePlugin::search_prefixes` and `rec_prefixes` lack documentation
**File:** `src/sources/plugin.rs` (lines 70–76)

These methods exist but are entirely undocumented. It's unclear when `rec_prefixes` (recommendations?) is used vs `search_prefixes`. Add doc comments explaining the expected format and usage.

---

---

## 📦 Binary Size Reduction


To audit what's actually being probed, add a `tracing::debug!` in `AudioProcessor::new` that logs `probed.format.name()`.

---

### S2. `reqwest` `blocking` feature adds a full synchronous HTTP stack
**File:** `Cargo.toml` (line 46)

The `blocking` feature compiles a separate thread-pool-based HTTP client on top of the async one. Since you're in a fully async Tokio app, this dead weight is compiled but shouldn't be used. Removing it saves compilation time and binary size:

```toml
reqwest = { version = "0.12", default-features = false, features = [
  "json", "rustls-tls", "stream", "cookies"
] }
```

---

### S3. `uuid` enables both `v4` and `v7` but only `v4` is used
**File:** `Cargo.toml` (line 45)

```toml
uuid = { version = "1.21.0", features = ["v4", "v7", "serde"] }
```

Search the codebase — only `Uuid::new_v4()` is called. Removing `"v7"` is a minor win since both pull in `rand`, but it signals exactly what the code needs:

```toml
uuid = { version = "1.21.0", features = ["v4", "serde"] }
```

---

### S4. `futures` crate is a large dependency — check if only specific sub-traits are needed
**File:** `Cargo.toml` (line 12)

The `futures` crate re-exports many things. In an async Tokio project, many `futures` utilities are available directly from `tokio::` or from smaller crates. Audit usages across the codebase and potentially replace with:
- `futures-util` (only combinators, no executor)
- Or rely on `tokio::stream`, `tokio::select!`, etc. directly

---

### S5. `davey = "0.1.1"` — unknown/obscure dependency
**File:** `Cargo.toml` (line 20)

`davey` is a very small, obscure crate with no known widespread adoption. Verify it is actually being used somewhere in the codebase. If it's a leftover dependency, remove it. Unused dependencies increase compile time, audit surface, and binary footprint.

---

### S6. Add a `[profile.dev]` section for faster development builds
**File:** `Cargo.toml`

The release profile is well-configured, but development builds have no configuration at all. For a codebase with this many crypto and audio processing dependencies, dev compile times can be very long. Add:

```toml
[profile.dev]
opt-level = 1         # basic optimization — dramatically speeds up debug builds
debug = true

[profile.dev.package."*"]
opt-level = 3         # fully optimize ALL dependencies even in dev mode
```

This alone can make `cargo build` **2–4× faster** during development by pre-optimizing heavy deps (symphonia, rubato, reqwest, etc.).

---

### S7. Enable `incremental = false` only in release; ensure it's on in dev
**File:** `Cargo.toml`

The release profile has `codegen-units = 1` which disables incremental compilation (correct for release). But make sure dev builds don't accidentally inherit this. Add explicitly:

```toml
[profile.dev]
incremental = true
```

---

---

## ⚡ Runtime Performance

### P1. Mixer `mix_buf` zeroing uses a manual loop instead of `fill()`
**File:** `src/audio/playback/mixer.rs` (lines 57–60)

```rust
// Before — manual loop
for s in self.mix_buf.iter_mut() {
    *s = 0;
}

// After — idiomatic, LLVM can auto-vectorize this
self.mix_buf.fill(0);
```

---

### P2. Mixer reads samples one-by-one via `try_recv()` in a tight loop
**File:** `src/audio/playback/mixer.rs` (lines 83–96)

Each call to `try_recv()` has per-call overhead (atomic check, potential contention). For a 1920-sample frame, this is 1920 individual atomic operations per track. Consider draining in chunks using `flume`'s `drain()` or switching the channel to send fixed-size frames (`[i16; 1920]`) instead of individual samples:

```rust
// Instead of sending i16 samples
flume::bounded::<i16>(4096 * 4);

// Send fixed frames
flume::bounded::<[i16; 960]>(16); // 16 frames buffer
```

This reduces channel overhead dramatically for the audio hot path.

---

### P3. `Resampler` sends samples one-by-one through a `flume::Sender`
**File:** `src/audio/pipeline/resampler.rs` (lines 43–46)

Each resampled sample is sent individually via `tx.send(s as i16)`. For a 48kHz stereo stream this is ~96,000 individual channel sends per second. Collect the resampled output into a `Vec<i16>` and send it in a single batch, or pass in a mutable output slice:

```rust
// Instead of:
pub fn process(&mut self, input: &[i16], tx: &Sender<i16>) -> AnyResult<()>

// Prefer:
pub fn process(&mut self, input: &[i16], output: &mut Vec<i16>) -> AnyResult<()>
```

---

### P4. `Ordering::SeqCst` used in audio hot path
**File:** `src/audio/playback/mixer.rs` (line 103), `src/player/playback.rs` (line 169)

`SeqCst` is the strongest (and most expensive) memory ordering. In the audio mixing loop, position updates and stop-signal checks don't need global sequential consistency — `Acquire`/`Release` pairs are sufficient and cheaper on x86 and ARM:

```rust
// Before
track.position.fetch_add((i / 2) as u64, Ordering::SeqCst);
stop_signal.load(Ordering::SeqCst);

// After
track.position.fetch_add((i / 2) as u64, Ordering::Relaxed); // position is only read for display
stop_signal.load(Ordering::Acquire);
```

---

### P5. `PlayerContext::to_player_response()` decodes the track on every call
**File:** `src/player/context.rs` (lines 59–64)

`Track::decode()` runs base64 decode + binary deserialization every time `to_player_response()` is called. This is called repeatedly in `start_playback` (multiple times per track start). Cache the decoded `TrackInfo` alongside the encoded `String` in `PlayerContext`:

```rust
pub struct PlayerContext {
    pub track: Option<String>,           // encoded
    pub track_info: Option<TrackInfo>,   // decoded — cache this
    ...
}
```

---

### P6. `FilterChain` boxes every filter with `Box<dyn AudioFilter>`
**File:** `src/audio/filters/mod.rs` (line 72)

```rust
filters: Vec<Box<dyn AudioFilter>>,
```

Dynamic dispatch through vtable calls in the audio hot path hurts performance and prevents inlining. Each filter call per sample frame crosses a vtable boundary. Consider an enum-dispatch approach:

```rust
pub enum ConcreteFilter {
    Volume(VolumeFilter),
    Equalizer(EqualizerFilter),
    Karaoke(KaraokeFilter),
    Tremolo(TremoloFilter),
    // ...
}
```

Or use the `enum_dispatch` crate which generates this automatically while keeping the trait API.

---

### P7. `timescale_buffer` grows unboundedly if frames aren't consumed
**File:** `src/audio/filters/mod.rs` (line 76, lines 209, 220–227)

`timescale_buffer` is a `Vec<i16>` that gets appended to every `process()` call, but is only drained when `fill_frame()` has enough data. Under a faster-than-realtime timescale (`speed > 1.0`), this buffer can grow without an upper bound. Add a capacity cap:

```rust
const MAX_TIMESCALE_BUFFER: usize = 1920 * 32; // max ~320ms of buffer

if self.timescale_buffer.len() > MAX_TIMESCALE_BUFFER {
    self.timescale_buffer.drain(..1920); // drop oldest frame
}
```

---

---

## 🪵 Logging Level Hygiene

The current logging has two problems: **too noisy at `info`** (floods production logs) and **not enough detail at `debug`/`trace`** (makes debugging hard).

### L1. Every REST endpoint logs at `info` — should be `debug`
**File:** `src/transport/routes/stats/info.rs`, `src/transport/routes/stats/track.rs`, `src/transport/routes/player/get.rs`, `src/transport/routes/player/update.rs`

Every HTTP handler fires an `info!` log on every request:

```rust
tracing::info!("GET /v4/info");
tracing::info!("GET /v4/loadtracks: identifier='{}'", identifier);
tracing::info!("GET /v4/sessions/{}/players", session_id);
tracing::info!("GET /v4/decodetrack");
tracing::info!("POST /v4/decodetracks: count={}", body.tracks.len());
```

These fire on **every client request** and pollute production logs. HTTP tracing is already provided by the `tower_http::trace::TraceLayer` applied in `main.rs`. Downgrade all of these to `debug!` or remove them entirely:

```rust
// Before
tracing::info!("GET /v4/loadtracks: identifier='{}'", identifier);

// After
tracing::debug!("GET /v4/loadtracks: identifier='{}'", identifier);
```

---

### L2. Source registration logs are fine at `info` — but should include the config summary
**File:** `src/sources/manager.rs` (lines 37–94)

The source registration logs (`info!("Registering YouTube source")`) are appropriate, but add no context about what was configured. Improve them to include relevant config details:

```rust
// Before
info!("Registering YouTube source");

// After
info!(
    "Registering YouTube source [search={:?}, playback={:?}]",
    config.youtube.clients.search,
    config.youtube.clients.playback
);
```

---

### L3. "No source could handle identifier" should be `debug`, not `warn`
**File:** `src/sources/manager.rs` (lines 119, 137)

```rust
tracing::warn!("No source could handle identifier: {}", identifier);
tracing::warn!("No source could handle search query: {}", query);
```

These fire whenever a source is checked and doesn't match. This is expected normal flow (e.g., Spotify source gets a YouTube URL). This is not a warning — it's informational routing. Change to `debug!`:

```rust
tracing::debug!("No source matched identifier: {}", identifier);
```

---

### L4. "Client connected without 'Client-Name'" is `warn` — should be `debug`
**File:** `src/transport/websocket_server.rs` (line 63)

```rust
warn!("Client connected without 'Client-Name' header");
```

Missing `Client-Name` is not a warning — it's an optional header. Many valid clients omit it. This fires on every connection without it, spamming production logs. Downgrade to `debug!`.

---

### L5. "Loading '…' with source" and mirror provider logs should be `trace`
**File:** `src/sources/manager.rs` (lines 114, 131, 178)

```rust
tracing::debug!("Loading '{}' with source: {}", identifier, source.name());
tracing::debug!("Attempting mirror provider: {}", search_query);
```

These fire on every single track load — at normal `debug` they'd flood debug sessions. Move to `trace!` since they're inner-loop diagnostic details:

```rust
tracing::trace!("Loading '{}' with source: {}", identifier, source.name());
tracing::trace!("Mirror provider attempt: {}", search_query);
```

---

### L6. WebSocket disconnect handling needs clearer log messages
**File:** `src/transport/websocket_server.rs` (lines 241–291)

The disconnect flow logs `info!("Connection closed (resumable)")` and `info!("Connection closed (not resumable)")` — both useful. But the resumption timeout expiry (`info!("Session resume timeout expired: {}", sid)`) should be `warn!`, as it means a session was lost without reconnection:

```rust
// This is a notable event worth flagging
warn!("Session resume timeout — session {} was not resumed in time and has been cleaned up.", sid);
```

---

### L7. Audio processor logs at `debug` for start/stop — should include track metadata
**File:** `src/audio/processor.rs` (lines 102–105) and `src/player/playback.rs` (line 118)

```rust
debug!("Starting playback loop: {}Hz {}ch -> {}Hz", self.source_rate, self.channels, self.target_rate);
info!("Playback starting: {} (source: {})", identifier, track_info.source_name);
```

These are good but could be improved:
- The `AudioProcessor` debug log should also include whether resampling is actually needed (`source_rate != target_rate`).
- The playback `info!` should log at `info` (which it does) — keep it.
- Add a `debug!` after successful decode setup to log codec info: `track.codec_params.codec`.

---

### L8. Token initialization and background tasks have no progress logging
**Files:** various source managers (Deezer, Spotify, Tidal)

Based on past conversations, token initialization happens in the background. There are no `debug!` logs for "token refresh started", "token refresh succeeded", or "token refresh failed". Add structured logging for these lifecycle events so token expiry can be diagnosed from logs without attaching a debugger.

---

---

## 📖 Documentation Improvements

### D1. `SourcePlugin` trait lacks a crate-level doc comment explaining the plugin model
**File:** `src/sources/plugin.rs`

The `SourcePlugin` trait is the extension point for all audio sources, but has no module-level `//!` comment explaining:
- How sources are registered (via `SourceManager::new`)
- What `can_handle` must guarantee (no side effects, fast, deterministic)
- The relationship between `load` (metadata) and `get_track` (playback URL)
- Thread-safety requirements (`Send + Sync`)

```rust
//! # Source Plugin System
//!
//! Sources are registered in [`SourceManager`] and consulted in order.
//! Each source must implement [`SourcePlugin`]:
//! - [`can_handle`] — fast, side-effect-free URL/identifier check
//! - [`load`] — resolve identifier into track metadata
//! - [`get_track`] — resolve identifier into a playable stream
```

---

### D2. `PlayableTrack::start_decoding` return tuple is hard to understand
**File:** `src/sources/plugin.rs` (lines 14–20)

The return type `(Receiver<i16>, Sender<DecoderCommand>, Receiver<String>)` has no named fields. Add a doc comment explaining each channel's role:

```rust
/// Start decoding and return three channels:
///
/// - `pcm_rx`: Receives interleaved i16 PCM samples at 48kHz stereo.
/// - `cmd_tx`: Send [`DecoderCommand::Seek`] or [`DecoderCommand::Stop`].
/// - `error_rx`: Receives at most one fatal error message. If the channel
///   closes cleanly (disconnected) without a message, decoding finished normally.
fn start_decoding(&self) -> (Receiver<i16>, Sender<DecoderCommand>, Receiver<String>);
```

---

### D3. `DecoderCommand` enum variants lack doc comments
**File:** `src/audio/processor.rs` (lines 16–19)

```rust
pub enum DecoderCommand {
    Seek(u64), // Position in milliseconds
    Stop,
}
```

The inline comment `// Position in milliseconds` should be a proper `///` doc comment. Also document that `Stop` causes the decode loop to exit gracefully (not abruptly):

```rust
pub enum DecoderCommand {
    /// Seek to the given position in milliseconds.
    /// Causes the resampler and decoder state to reset.
    Seek(u64),
    /// Gracefully stop the decode loop. The PCM channel will be dropped after this.
    Stop,
}
```

---

### D4. `AudioFilter` trait needs method-level docs
**File:** `src/audio/filters/mod.rs` (lines 61–68)

`process`, `is_enabled`, and `reset` have no individual method docs beyond the trait-level comment. Specifically:
- `is_enabled` — document what "enabled" means (i.e., non-identity/non-default params)
- `reset` — document when this is called (seek, filter config change)
- `process` — document the expected buffer layout (1920 interleaved stereo i16 samples)

---

### D5. `FilterChain::fill_frame` behavior on underrun is undocumented from caller's perspective
**File:** `src/audio/filters/mod.rs` (lines 215–227)

The function returns `false` when there aren't enough timescale-buffered samples yet. The caller's expected behavior in this case (skip the frame, output silence?) is not documented. Add:

```rust
/// Drain exactly `output.len()` resampled samples into `output`.
///
/// Returns `true` if enough data was available (frame written).
/// Returns `false` if the timescale buffer is underflowed — the caller
/// should output silence for this frame and try again next tick.
pub fn fill_frame(&mut self, output: &mut [i16]) -> bool {
```

---

### D6. `Mixer` struct and fields need doc comments
**File:** `src/audio/playback/mixer.rs`

`Mixer` and `MixerTrack` have no doc comments. Document:
- What `mix_buf` is (intermediate i32 accumulation buffer to prevent clipping before converting back to i16)
- The expected frame size (1920 samples = 960 stereo frames = 20ms at 48kHz)
- Why `i32` is used as the accumulation type (to hold the sum of multiple i16 tracks without overflow)

---

### D7. `PlayerContext` fields lack doc comments for non-obvious ones
**File:** `src/player/context.rs` (lines 14–33)

Fields like `stop_signal`, `frames_sent`, `frames_nulled`, `ping` have no docs explaining:
- `stop_signal`: used to break the `track_task` loop when a new track starts
- `frames_sent` / `frames_nulled`: used for Discord voice stats reporting
- `ping`: updated by the gateway module from voice UDP heartbeats

---

### D8. `build.rs` fallback logic for git info is undocumented
**File:** `build.rs` (lines 66–92)

The file has two code paths: try `git` CLI, fall back to reading `.git/HEAD` manually. Add a top-level comment:

```rust
// Build script that embeds git metadata (branch, commit SHA, timestamp)
// into the binary via environment variables at compile time.
//
// Tries the `git` CLI first. Falls back to direct .git/ file parsing
// for environments where git is not available (e.g. in some CI/Docker builds).
```

---

*Last updated: 2026-02-22*
