# Rustalink — Code Improvement Suggestions

Organized by impact area. Every item references the exact file and line. All audio
improvements are **post-decode PCM only** — Symphonia is untouched.

---

## 🔴 Critical — Correctness & Memory Safety

### C1. `MirroredTrack` creates a new Tokio runtime per track (DONE)
**File:** `src/sources/manager.rs` lines 278–315

**Problem:** Every time a mirrored track starts, `std::thread::spawn` is called
and inside it `tokio::runtime::Builder::new_current_thread().build()` creates a
fresh runtime. Each runtime allocates its own thread pool, timer wheel, and I/O
driver. Under concurrent load this can create hundreds of runtimes, leaking
memory and file descriptors that are never properly cleaned up.

**Fix:** Use `tokio::task::spawn` directly — the tokio runtime is already running.
```rust
// Before — src/sources/manager.rs line 278
std::thread::spawn(move || {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all().build().unwrap();
    runtime.block_on(async { ... });
});

// After — uses the existing runtime, zero extra allocation
tokio::spawn(async move {
    if let LoadResult::Search(tracks) = manager.load(&query, None).await {
        ...
    }
});
```

---

### C2. `reqwest` `blocking` feature is live inside an async runtime (DONE)
**File:** `Cargo.toml`

**Problem:** `reqwest`'s blocking client internally calls `thread::park` which
panics when used inside a Tokio worker thread. The `blocking` feature also
compiles a full second HTTP stack (+300 KB binary). If any source calls
`reqwest::blocking::*` inside an `async fn`, it will panic at runtime.

**Fix:**
```toml
# Cargo.toml — remove "blocking"
reqwest = { version = "0.12", features = ["json", "rustls-tls", "stream", "cookies"] }
```

---

### C3. Mixer acquires `Mutex<FilterChain>` AND `Mutex<DaveHandler>` per frame in the speak loop (DONE)
**File:** `src/gateway/session.rs` lines 814–859

**Problem:** The speak loop runs every 20 ms. Each iteration:
1. Locks `filter_chain` (Tokio Mutex — yields if contended)
2. Locks `dave` (Tokio Mutex — yields if contended)
3. Sends a UDP packet while `dave` is still locked

Any lock contention on step 1 or 2 delays the UDP send, causing audio glitches.
Holding the `dave` lock during I/O is especially bad — UDP `send_to` can block
momentarily on a full socket buffer.

**Fix:** Copy the encrypted packet bytes out while `dave` is locked, then send
after dropping the lock:
```rust
let encrypted = {
    let mut dave = dave.lock().await;
    dave.encrypt_opus(&opus_buf[..size])? // returns Vec<u8>
};
// dave lock is released here
udp.send_opus_packet(&encrypted)?;
```

---

### C4. `Mutex::lock().unwrap()` panics on mutex poisoning
**Files:** `src/common/logger.rs`, `src/sources/manager.rs` (multiple)

**Problem:** If a thread panics while holding a `Mutex`, the mutex becomes
"poisoned". Any future `.lock().unwrap()` will then panic that thread too,
cascading into a full server crash.

**Fix:**
```rust
// Instead of
let state = self.state.lock().unwrap();

// Use poison-safe recovery
let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
```

---

### C5. `timescale_buffer` grows without bound under `speed > 1.0` (DONE)
**File:** `src/audio/filters/mod.rs` lines 76, 207–210

**Problem:** `timescale_buffer` is a `Vec<i16>` that is extended on every
`process()` call but only drained when `fill_frame()` succeeds. When timescale
`speed > 1.0`, the filter produces more samples per input frame than the speak
loop consumes. The buffer grows indefinitely, eventually exhausting heap memory.

**Fix:** Cap the buffer and drop old frames:
```rust
// src/audio/filters/mod.rs — after line 209
const MAX_TS_BUFFER: usize = 1920 * 64; // ~640ms
if self.timescale_buffer.len() > MAX_TS_BUFFER {
    let drop_count = self.timescale_buffer.len() - MAX_TS_BUFFER;
    self.timescale_buffer.drain(..drop_count);
}
```

---

### C6. `SourceManager::new` uses `expect()` for non-fatal source initialization (DONE)
**File:** `src/sources/manager.rs` line 47

```rust
DeezerSource::new(...).expect("Failed to create Deezer source")
```

**Problem:** If Deezer config is invalid, the entire server crashes at startup.
**Fix:** Return `Result<Self>` from `SourceManager::new`, or log and skip:
```rust
match DeezerSource::new(config.deezer.clone().unwrap_or_default()) {
    Ok(src) => sources.push(Box::new(src)),
    Err(e) => tracing::error!("Deezer source disabled: {}", e),
}
```

---

## 🟠 High — Performance (Audio Hot Path)

### H1. Resampler sends 96,000 individual samples per second through a channel (DONE)
**File:** `src/audio/pipeline/resampler.rs` lines 44–46

**Problem:** `tx.send(s as i16)` is called for every output sample. At 48kHz
stereo this is ~96,000 channel sends per second. Each send involves an atomic
CAS, potential cache-line bounce, and scheduler yield. This is the single
largest allocation-free overhead in the entire audio path.

**Fix:** Collect into a `Vec` and send as a batch (or better: pass a mutable
output slice so there's zero allocation):
```rust
// Option A — batch send (one allocation per decoded frame)
pub fn process(&mut self, input: &[i16], output: &mut Vec<i16>) {
    output.clear();
    while self.index < num_frames as f64 {
        // ... interpolation ...
        for c in 0..self.channels {
            output.push(interpolated_sample as i16);
        }
        self.index += self.ratio;
    }
}

// Option B — change channel type to send fixed frames
// flume::bounded::<[i16; 960]>(16) — 16 frames buffer
```

---

### H2. Mixer reads each sample individually via `try_recv()` in a tight loop (DONE)
**File:** `src/audio/playback/mixer.rs` lines 83–96

**Problem:** `try_recv()` is called 1920 times per frame per track, performing
1920 atomic load operations per 20ms tick. For 10 concurrent guild players this
is 19,200 atomic operations every 20ms just to read audio data.

**Fix:** Change the channel type to send fixed-size frames:
```rust
// In start_decoding: change channel element type
let (tx, rx) = flume::bounded::<Box<[i16; 960]>>(16);

// In mixer — one recv per frame, zero per-sample overhead
while let Ok(frame) = track.rx.try_recv() {
    for (i, &s) in frame.iter().enumerate() {
        self.mix_buf[i] += (s as f32 * vol) as i32;
    }
}
```

---

### H3. `mix_buf` zeroing uses a manual loop instead of SIMD-optimizable `fill` (DONE)
**File:** `src/audio/playback/mixer.rs` lines 57–60

```rust
// Before — LLVM cannot always autovectorize this pattern
for s in self.mix_buf.iter_mut() { *s = 0; }

// After — LLVM recognizes fill() and emits SIMD memset
self.mix_buf.fill(0);
```

---

### H4. `Ordering::SeqCst` in the audio hot path
**File:** `src/audio/playback/mixer.rs` line 103

```rust
// Before — SeqCst forces a full memory barrier on every call (expensive on ARM)
track.position.fetch_add((i / 2) as u64, Ordering::SeqCst);

// After — position is only read for UI display, Relaxed is sufficient
track.position.fetch_add((i / 2) as u64, Ordering::Relaxed);
```

Similarly in `src/gateway/session.rs` line 321:
```rust
// Before
last_hb_inner.store(now as i64, Ordering::SeqCst);
// After
last_hb_inner.store(now as i64, Ordering::Release);
```

---

### H5. `FilterChain` uses `Box<dyn AudioFilter>` — vtable dispatch per sample frame (DONE)
**File:** `src/audio/filters/mod.rs` line 72

**Problem:** `Vec<Box<dyn AudioFilter>>` means every `filter.process(samples)`
call goes through a vtable indirect jump. The CPU cannot inline or speculatively
execute the filter body, and branch prediction fails on vtable calls. With 8
filters active and 50 guild players, this is 400 vtable calls per 20ms tick.

**Fix:** Replace with a concrete enum to enable inlining:
```rust
pub enum ConcreteFilter {
    Volume(volume::VolumeFilter),
    Equalizer(equalizer::EqualizerFilter),
    Karaoke(karaoke::KaraokeFilter),
    Tremolo(tremolo::TremoloFilter),
    Vibrato(vibrato::VibratoFilter),
    Rotation(rotation::RotationFilter),
    Distortion(distortion::DistortionFilter),
    ChannelMix(channel_mix::ChannelMixFilter),
    LowPass(low_pass::LowPassFilter),
}

impl ConcreteFilter {
    #[inline(always)]
    pub fn process(&mut self, samples: &mut [i16]) { ... }
}
```
Or use the `enum_dispatch` crate to generate this automatically without
changing the `AudioFilter` trait API.

---

### H6. `speak_loop` uses `MissedTickBehavior::Burst` — can flood the scheduler (DONE)
**File:** `src/gateway/session.rs` line 782

**Problem:** `Burst` re-fires all missed 20ms ticks as fast as possible after a
pause. If the server pauses for 200ms (GC, load spike), this sends 10 UDP
packets in rapid succession, potentially overflowing the socket send buffer and
causing real packet loss. Lavalink uses `Skip` — drop missed ticks and send
silence frames instead.

**Fix:**
```rust
interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
```

---

### H7. `PlayerContext::to_player_response()` decodes the base64 track on every call (DONE)
**File:** `src/player/context.rs` line 61

```rust
crate::api::tracks::Track::decode(t)  // base64 + binary deserialization each time
```

This runs on every `GET /v4/sessions/{id}/players` request and every `PlayerUpdate`
WebSocket message. Cache the decoded `TrackInfo` alongside the encoded string:

```rust
pub struct PlayerContext {
    pub track: Option<String>,         // encoded (for serialization)
    pub track_info: Option<TrackInfo>, // decoded (cached — avoid re-parsing)
    ...
}
```

---

### H8. `AudioProcessor` clones the `Sender<i16>` on every decoded frame (DONE)
**File:** `src/audio/processor.rs` line 144

```rust
let tx = self.tx.clone(); // clone inside the packet loop
self.resampler.process(samples, &tx)?;
```

`flume::Sender` clone increments an `Arc` refcount. Move the clone out of
the hot loop — keep one reference in the `Resampler` call by passing `&self.tx`
directly (once the resampler signature changes per H1 above, this disappears
entirely).

---

## 🟡 Medium — Code Structure

### M1. `GuildId` / `SessionId` are plain `String` aliases — no type safety (DONE)
**File:** `src/common/types.rs` lines 18–19

```rust
pub type GuildId = String;    // can accidentally pass SessionId here
pub type SessionId = String;
```

**Fix:** Newtype wrappers make misuse a compile error:
```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct GuildId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct SessionId(pub String);
```

---

### M2. `HttpClient` is a zero-field struct used only as a namespace (DONE)
**File:** `src/common/http.rs`

A struct with no fields and only associated functions is an anti-pattern in
Rust. Move them to module-level free functions:
```rust
// Before
HttpClient::new()
HttpClient::default_user_agent()

// After — idiomatic Rust
pub const DEFAULT_USER_AGENT: &str = "Mozilla/5.0 ...";
pub fn new_http_client() -> reqwest::Result<Client> { ... }
```

---

### M3. `SourceManager::new` is a 70-line `if` chain — hard to extend (DONE)
**File:** `src/sources/manager.rs` lines 39–101

Each new source requires manually adding an `if config.sources.X { ... }` block.
This pattern doesn't scale and makes it easy to forget to wire a source.

**Fix:** Use a registration table pattern:
```rust
macro_rules! register_source {
    ($enabled:expr, $name:literal, $ctor:expr, $sources:expr) => {
        if $enabled {
            match $ctor {
                Ok(s) => {
                    tracing::info!("Registering {} source", $name);
                    $sources.push(Box::new(s) as BoxedSource);
                }
                Err(e) => tracing::error!("{} source disabled: {}", $name, e),
            }
        }
    };
}
```
Or define a `SourceFactory` trait with a `build(&Config) -> Option<BoxedSource>` method per source.

---

### M4. `validate_filters` allocates a `Vec<String>` for every filter check (DONE)
**File:** `src/audio/filters/mod.rs` lines 22–57

`validate_filters` pushes owned `String`s even though the names are all
`'static` string literals. Use `&'static str` to avoid heap allocation:
```rust
pub fn validate_filters(filters: &Filters, config: &FiltersConfig) -> Vec<&'static str> {
    let mut invalid = Vec::new();
    if filters.volume.is_some() && !config.volume { invalid.push("volume"); }
    // ... etc
    invalid
}
```

---

### M5. `PlayerUpdate.end_time` is `Option<Option<u64>>` — confusing triple-state (DONE)
**File:** `src/player/state.rs` line 69

`Option<Option<u64>>` means three things: "not provided", "clear", "set value".
This is confusing and easy to mishandle with pattern matching.

**Fix:** A dedicated enum is self-documenting and exhaustive:
```rust
#[derive(Debug, Default, Deserialize)]
#[serde(untagged)]
pub enum EndTime {
    #[default]
    Unchanged,
    Clear,           // JSON: null
    Set(u64),        // JSON: number
}
```

---

### M6. `configs/sources.rs` has dozens of single-line `fn default_xxx()` functions (DONE)
**File:** `src/configs/sources.rs`

Each default value is its own function like `fn default_search_limit() -> usize { 10 }`.
This adds ~5 lines per config field and scatters defaults across the file.

**Fix:** Consolidate into `impl Default` with inline values, or use Serde's
`default = "literal"` syntax (Rust 1.77+):
```rust
impl Default for SpotifyConfig {
    fn default() -> Self {
        Self {
            playlist_load_limit: 6,
            album_load_limit: 6,
            search_limit: 10,
        }
    }
}
```

---

### M7. `AudioProcessor::check_commands` recreates a `Resampler` on every seek (DONE)
**File:** `src/audio/processor.rs` line 189

```rust
self.resampler = Resampler::new(self.source_rate, self.target_rate, self.channels);
```

This allocates a new `Vec` for `last_samples` on every seek. Instead, add a
`reset()` method to `Resampler` that zeros `index` and `last_samples` in place:
```rust
impl Resampler {
    pub fn reset(&mut self) {
        self.index = 0.0;
        self.last_samples.fill(0);
    }
}
// Then in check_commands:
self.resampler.reset();
```

---

### M8. `DecoderCommand::Seek` return value is unused (DONE)
**File:** `src/audio/processor.rs` lines 193, 109–113

`check_commands()` returns `Some(DecoderCommand::Seek(ms))` but the caller
never inspects the value — it only checks `if matches!(cmd, DecoderCommand::Stop)`.
The `Seek` variant is handled entirely inside `check_commands`. Return a simpler
type:
```rust
enum CommandOutcome {
    Stop,
    Seeked,
    None,
}
fn check_commands(&mut self) -> CommandOutcome { ... }
```

---

## 🟢 Low — Polish, Binary Size, Build Speed

### B1. Development builds have no `[profile.dev]` — compile times are long (DONE)
**File:** `Cargo.toml`

Add a dev profile to pre-optimize heavy dependencies (symphonia, rubato,
reqwest) while keeping your own code unoptimized for fast incremental builds:
```toml
[profile.dev]
opt-level = 1
incremental = true
debug = true

[profile.dev.package."*"]
opt-level = 3   # fully optimize all external crates in dev mode
```
This typically makes `cargo build` **2–4× faster** in practice.

---

### B2. `uuid` enables `v7` but only `v4` is used (DONE)
**File:** `Cargo.toml` line 45

```toml
# Before
uuid = { version = "1.21.0", features = ["v4", "v7", "serde"] }
# After
uuid = { version = "1.21.0", features = ["v4", "serde"] }
```

---

### B3. `futures` is imported as a full crate — use sub-crates instead
**File:** `Cargo.toml` line 12

The `futures` crate includes an executor. In a Tokio project, most needed types
(`Stream`, `Sink`, `StreamExt`, `SinkExt`) are in `futures-util` (no executor):
```toml
futures-util = { version = "0.3", default-features = false }
```

---

### B4. `source_names()` allocates a `Vec<String>` for a debug helper
**File:** `src/sources/manager.rs` lines 246–248

```rust
// Before — new String allocation for every source name
pub fn source_names(&self) -> Vec<String> {
    self.sources.iter().map(|s| s.name().to_string()).collect()
}

// After — zero allocation, caller borrows
pub fn source_names(&self) -> impl Iterator<Item = &str> {
    self.sources.iter().map(|s| s.name())
}
```

---

### B5. `strip_ansi_escapes` in logger is hand-rolled and incorrect
**File:** `src/common/logger.rs` lines 46–61

The manual stripper only handles single-letter terminators. It fails on:
- `ESC[38;2;255;128;0m` (24-bit RGB)
- `ESC[1;32m` (combined attributes)
- `ESC]` OSC sequences

Use the `strip-ansi-escapes` crate instead:
```toml
strip-ansi-escapes = "0.2"
```

---

### B6. `CircularFileWriter::write` holds the Mutex lock during file I/O
**File:** `src/common/logger.rs` lines 176–199

File writes can block for milliseconds. Holding a Mutex across a file write
blocks every other logger call. Release the lock before pruning:
```rust
let should_prune = {
    let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
    state.lines_since_prune += new_lines;
    if state.lines_since_prune >= PRUNE_THRESHOLD {
        state.lines_since_prune = 0;
        true
    } else { false }
}; // lock released here
if should_prune { let _ = self.prune(); }
```

---

## 🔊 Voice Gateway Improvements

### V1. `is_reconnectable_close` is missing Discord disconnect codes (DONE)
**File:** `src/gateway/session.rs` line 32

```rust
fn is_reconnectable_close(code: u16) -> bool {
    matches!(code, 1006 | 4015 | 4009)
}
```

Missing cases that Lavalink handles:
- `4006` — session no longer valid → reconnect with fresh Identify (not Resume)
- `4014` — channel deleted / bot disconnected → **stop** (do not reconnect)
- `4004` — authentication failed → **stop** (reconnecting with same token loops forever)

**Fix:**
```rust
fn is_reconnectable_close(code: u16) -> bool {
    matches!(code, 1006 | 4009 | 4015)
}

fn is_fatal_close(code: u16) -> bool {
    matches!(code, 4004 | 4014)
}
```

---

### V2. UDP socket is a blocking `std::net::UdpSocket` inside an async task (DONE)
**File:** `src/gateway/session.rs` lines 237–238, `speak_loop` line 779

The UDP socket bound in `run_session` is `std::net::UdpSocket` (blocking).
It is then `try_clone()`d into `UdpBackend`. A blocking send/recv blocks the
entire Tokio worker thread. Replace with `tokio::net::UdpSocket`:
```rust
// Before
let udp_socket = UdpSocket::bind("0.0.0.0:0")?;

// After
let udp_socket = tokio::net::UdpSocket::bind("0.0.0.0:0").await?;
```

---

### V3. Heartbeat measures latency using `SystemTime`, which can jump backwards
**File:** `src/gateway/session.rs` lines 317–320, 464–468

`SystemTime::now()` is not monotonic — NTP adjustments can make it go
backwards, resulting in negative latency values:
```rust
// Before
let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;

// After — use Instant for elapsed, SystemTime only for the wire timestamp
let sent = std::time::Instant::now();
// ...on ACK:
self.ping.store(sent.elapsed().as_millis() as i64, Ordering::Relaxed);
```

---

## 🪵 Logging Hygiene

### L1. Every REST endpoint fires `info!` on every request
**Files:** `src/transport/routes/stats/info.rs`, `src/transport/routes/player/get.rs`, etc.

`tower_http::TraceLayer` already traces every HTTP request. Downgrade these
to `debug!` or remove them. Keeping them at `info!` floods production logs.

---

### L2. "No source could handle identifier" logs at `warn!`
**File:** `src/sources/manager.rs` lines 124, 142

This fires during normal routing (e.g. Spotify source checking a YouTube URL).
It is **not** a warning. Change to `debug!`.

---

### L3. "Client connected without 'Client-Name'" logs at `warn!`
**File:** `src/transport/websocket_server.rs` line 63

`Client-Name` is optional in the Lavalink spec. Downgrade to `debug!`.

---

### L4. Source loading inner loop logs at `debug!` — should be `trace!`
**File:** `src/sources/manager.rs` lines 119, 136, 155–158, 183

```rust
// Before — fires on every track load, every mirror attempt
tracing::debug!("Loading '{}' with source: {}", identifier, source.name());

// After
tracing::trace!("Loading '{}' with source: {}", identifier, source.name());
```

---

### L5. Session resume timeout should be `warn!`, not `info!`
**File:** `src/transport/websocket_server.rs`

A session that times out without resuming means a guild player was silently
destroyed. This is worth a `warn!` in production.

---

## 📖 Documentation

### D1. `PlayableTrack::start_decoding` return tuple has no named semantics (DONE)
**File:** `src/sources/plugin.rs`

The return type `(Receiver<i16>, Sender<DecoderCommand>, Receiver<String>)` is
opaque. Add a doc comment:
```rust
/// Returns three channels:
/// 0. `pcm_rx`   — interleaved i16 PCM at 48kHz stereo
/// 1. `cmd_tx`   — send `Stop` or `Seek(ms)` commands
/// 2. `error_rx` — receives at most one fatal error String;
///                 a clean disconnect means normal end-of-track
```

---

### D2. `AudioFilter::process` buffer layout is undocumented (DONE)
**File:** `src/audio/filters/mod.rs` line 63

Add to the trait:
```rust
/// Process samples **in-place**.
///
/// `samples` is an interleaved stereo i16 buffer:
///   `[L₀, R₀, L₁, R₁, ...]` — typically 1920 elements (960 frames × 2 ch = 20ms at 48kHz).
fn process(&mut self, samples: &mut [i16]);
```

---

### D3. `FilterChain::fill_frame` return value behavior on underrun is not documented
**File:** `src/audio/filters/mod.rs` lines 215–227

```rust
/// Returns `true` if a full frame was drained into `output`.
/// Returns `false` if the timescale buffer contains fewer than `output.len()` samples.
/// The caller should output silence for this tick and retry next frame.
pub fn fill_frame(&mut self, output: &mut [i16]) -> bool {
```

---

### D4. `Mixer` accumulation buffer type rationale is undocumented
**File:** `src/audio/playback/mixer.rs` line 10

```rust
// Add above the field:
/// i32 accumulation buffer: adding multiple i16 tracks would overflow i16
/// (max value 32767 × N tracks). Clamped back to i16 in the output pass.
mix_buf: Vec<i32>,
```

---

### D5. `build.rs` fallback logic for git info has no top-level comment
**File:** `build.rs`

```rust
// Build script: embeds git branch, commit SHA, and build timestamp into the
// binary via CARGO_PKG_* env vars at compile time.
//
// Strategy:
//   1. Try `git rev-parse` / `git log` via CLI.
//   2. Fall back to reading .git/HEAD directly (for Docker/CI with no git binary).
```

---

## 🔒 Security & Stability

### SEC1. No rate limiting on REST or WebSocket endpoints
**Files:** `src/transport/http_server.rs`, `src/transport/websocket_server.rs`

Any client can flood `/v4/loadtracks` with thousands of requests per second.
Add a simple per-IP sliding window rate limiter as an Axum `tower::Layer`:
```toml
# Cargo.toml
tower = { version = "0.4", features = ["limit"] }
```

---

### SEC2. No DoS burst protection on WebSocket connections
**File:** `src/transport/websocket_server.rs`

Without a connection limit, a bad actor can open thousands of WebSocket
connections, each demanding a DAVE E2EE session. Use a global `AtomicUsize`
connection counter and reject connections above a configured threshold.

---

### SEC3. Voice gateway reconnect loop has no jitter — all clients retry simultaneously
**File:** `src/gateway/session.rs` lines 133, 151

```rust
let backoff = Duration::from_millis(1000 * 2u64.pow((attempt - 1).min(3)));
```

Under a Discord outage, all guilds retry at the same exponential intervals,
causing another thundering herd. Add random jitter:
```rust
use rand::Rng;
let jitter = rand::thread_rng().gen_range(0..500);
let backoff = Duration::from_millis(1000 * 2u64.pow((attempt - 1).min(3)) + jitter);
```

---

## 🎵 Audio Pipeline Extensions (PCM-only, no Symphonia changes)

All items below operate on the decoded i16 PCM buffer after Symphonia finishes
decoding. They add zero risk to the decoding pipeline.

### A1. Add EBU R128 loudness normalization
**Reference:** `NodeLink-dev/src/playback/processing/LoudnessNormalizer.ts`

Prevents dramatic volume jumps between tracks (quiet classical → loud EDM).
Implemented as a `LoudnessNormalizer` struct that runs **after** `FilterChain::process()`,
**before** `encoder.encode()` in the speak loop. Uses K-Weighting biquad filters
(the coefficients are a well-known standard) and an energy gate to avoid boosting
silence gaps.

Key parameters: `target_lufs` (default -14 LUFS, matching Spotify/YouTube), `attack_ms`, `release_ms`.

---

### A2. Add crossfade between tracks
**Reference:** `NodeLink-dev/src/playback/processing/CrossfadeController.ts`

Replaces the hard cut between tracks with a configurable overlap mix.
The next track's PCM is decoded into a `RingBuffer` while the current track
plays. When the current track ends, the speak loop mixes both buffers using
constant-power sinusoidal gain curves. No Symphonia involvement whatsoever.

Config addition: `crossfade_duration_ms` in `PlayerUpdate`.

---

### A3. Add Reverb filter (Freeverb algorithm)
**Reference:** `NodeLink-dev/src/playback/filters/reverb.ts`

8 parallel comb filters + 4 allpass stages, parametrized by `mix`, `room_size`,
`damping`, `width`. Operates entirely on the i16 PCM buffer via the existing
`AudioFilter` trait. No external crate required — all math is simple multiply-add.

New file: `src/audio/filters/reverb.rs`

---

### A4. Add Chorus filter
**Reference:** `NodeLink-dev/src/playback/filters/chorus.ts`

4 LFO-modulated delay lines (2 per stereo channel) with `rate`, `depth`, `delay`,
`mix`, `feedback`. The `LFO` and `DelayLine` helpers already exist in
`src/audio/filters/lfo.rs` and `src/audio/filters/delay_line.rs`. This is mostly
a wiring exercise.

---

### A5. Add Echo / Delay filter
**Reference:** `NodeLink-dev/src/playback/filters/echo.ts`

Simple feedback echo: `out = dry*in + wet*delay_buffer[delay_samples]`.
Parameters: `delay_ms`, `decay`, `mix`. Popular for karaoke and effect pads.

---

## 📊 Observability

### O1. Add optional Prometheus metrics endpoint
**Reference:** Lavalink-master Prometheus integration

```toml
# Cargo.toml (feature-gated)
metrics = { version = "0.23", optional = true }
metrics-exporter-prometheus = { version = "0.15", optional = true }
```
```toml
# config.toml
[metrics]
enabled = false
endpoint = "/metrics"
```

Expose: `players`, `playing_players`, `frames_sent`, `frames_nulled`,
`voice_connections`, `track_loads_total` (per source), `track_load_duration_ms`.

---

### O2. Add per-source track load duration logging
**File:** `src/sources/manager.rs` — inside `load()`

```rust
let start = std::time::Instant::now();
let result = source.load(identifier, routeplanner.clone()).await;
tracing::debug!(
    source = source.name(),
    duration_ms = start.elapsed().as_millis(),
    "track load complete"
);
result
```

---

### O3. Ensure `frames_sent` / `frames_nulled` are correctly reported in `/v4/stats`
**File:** `src/transport/routes/stats/`

Per the Lavalink v4 spec, `/v4/stats` must include `frameStats`:
```json
{
  "frameStats": {
    "sent": 6000,
    "nulled": 10,
    "deficit": -3010
  }
}
```
`deficit = sent + nulled - expected` where `expected = connected_players × 3000` (3000 frames/min).

---

*Last updated: 2026-02-22*
