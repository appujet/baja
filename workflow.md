# baja Audio Pipeline — Implementation Workflow

> Ordered implementation plan for building out the full audio pipeline in Rust.
> Each phase is self-contained and testable before moving to the next.

---

## Phase 0 — Foundations

These must exist before any pipeline work begins.

### 0.1  `audio/constants.rs`
Define all shared constants in one place:
```rust
pub const FRAME_SIZE: usize = 3_840;   // bytes per PCM frame (960 * 2ch * 2B = 20 ms)
pub const FRAME_SAMPLES: usize = 960;   // samples per frame per channel
pub const SAMPLE_RATE: u32 = 48_000;
pub const CHANNELS: usize = 2;
pub const AAC_RING_BYTES: usize = 2 * 1024 * 1024;   // default AAC ring buffer
pub const PCM_FLOAT_FACTOR: f32 = 32_767.0;
```
Add `pub mod constants;` to `audio/mod.rs`.

### 0.2  `audio/buffer.rs` — audit
- `PooledBuffer` must pool sizes aligned to powers of two (min 1 024 B).
- Pool cap: 50 MB total, max 8 entries per bucket (or configurable via env).
- `PooledBuffer` must implement `Drop` so buffers return automatically.
- Add `BufferPool::stats()` for monitoring.

---

## Phase 1 — DSP Primitives Reorganisation

Move shared DSP internals out of `filters/` root.

### 1.1  Create `audio/filters/dsp/`
```
audio/filters/dsp/
├── mod.rs          # pub use all
├── biquad.rs       # move from filters/biquad.rs
├── lfo.rs          # move from filters/lfo.rs
├── delay_line.rs   # move from filters/delay_line.rs
└── clamp.rs        # NEW: clamp_i16(v: i32) -> i16, clamp_f32(v: f32) -> f32
```
Update all filter files that `use super::biquad` to `use crate::audio::filters::dsp::biquad`.

### 1.2  Extract `FilterChain` from `filters/mod.rs`
`filters/mod.rs` is 16 KB — move `FilterChain` impl to `filters/chain.rs`:
```rust
// filters/chain.rs
pub struct FilterChain { filters: Vec<Box<dyn AudioFilter>> }
impl FilterChain {
    pub fn from_config(f: &Filters) -> Self { ... }
    pub fn process(&mut self, buf: &mut [i16]) { ... }
    pub fn reset(&mut self) { ... }
}
```
`filters/mod.rs` becomes re-exports only.

### 1.3  Timescale submodule
Split `filters/timescale.rs` (~4.6 KB) into:
```
filters/timescale/
├── mod.rs           # TimescaleFilter: AudioFilter impl
├── time_stretch.rs  # WSOLA overlap-add core
└── resampler.rs     # Rate-only conversion (no pitch shift)
```

---

## Phase 2 — Effects Layer

Build the per-frame effects that run inside the mixer on every 20 ms frame.

### 2.1  `audio/playback/effects/tape.rs` — audit existing
Current `TapeEffect` uses `TapeState` enum. Ensure it:
- Uses Cubic Hermite interpolation (4-point: p0, p1, p2, p3).
- Supports curves: `linear | exponential | sinusoidal` (default sinusoidal).
- Exposes `tape_to(duration_ms, type: Start | Stop, curve)`.
- Exposes `check_ramp_completed() -> bool` (one-shot flag, clears on read).
- Maintains a `Float32` sliding-window buffer sized to 10 × SAMPLE_RATE × CHANNELS.
- Has `compact()` to shift the window when `read_pos > 2 × SAMPLE_RATE × CHANNELS`.

### 2.2  `audio/playback/effects/fade.rs`  `[NEW]`
```rust
pub struct FadeTransformer {
    gain: f32,
    target_gain: f32,
    duration_samples: usize,
    elapsed_samples: usize,
    curve: FadeCurve,
}

pub enum FadeCurve { Linear, Sine, Sinusoidal }

impl FadeTransformer {
    pub fn set_gain(&mut self, gain: f32);
    pub fn fade_to(&mut self, target: f32, duration_ms: u32, curve: FadeCurve);
    pub fn process(&mut self, buf: &mut [i16]);
}
```

### 2.3  `audio/playback/effects/volume_ramp.rs`  `[NEW]`
```rust
pub fn apply_ramp(buf: &mut [i16], gain_start: f32, gain_end: f32);
pub fn curve_value(t: f32, curve: FadeCurve) -> f32;
// linear:      t
// exponential: t*t
// sinusoidal:  (1 - cos(t * PI)) / 2
```

### 2.4  `audio/filters/volume.rs` — expand
Add sinusoidal fade-curves and AGC limiter integration using `normalization.rs`.

---

## Phase 3 — Flow Controller

The `FlowController` ensures every 20 ms frame passes through effects in the correct order.

### 3.1  `audio/playback/flow.rs`  `[NEW]`
```rust
pub struct FlowController {
    filter_chain: Arc<Mutex<FilterChain>>,
    tape: TapeEffect,
    volume: VolumeFilter,
    fade: FadeTransformer,
    // AudioMixer is optional (for mix layers)
    mixer: Option<Arc<Mutex<Mixer>>>,
    pending: [i16; FRAME_SAMPLES * CHANNELS],
    pending_len: usize,
}

impl FlowController {
    // Called per FRAME_SIZE bytes in exact order:
    fn process_frame(&mut self, frame: &mut [i16; FRAME_SAMPLES * CHANNELS]) {
        self.filter_chain.lock().process(frame);  // 1. DSP filters
        self.tape.process(frame);                  // 2. Tape ramp
        self.volume.process(frame);                // 3. Volume + fade
        self.fade.process(frame);                  // 4. Fade envelope
        if let Some(mixer) = &self.mixer {         // 5. Mix layers
            mixer.lock().mix_layers_into(frame);
        }
    }

    pub fn set_filters(&mut self, f: &Filters);
    pub fn set_volume(&mut self, v: f32);
    pub fn fade_to(&mut self, target: f32, ms: u32, curve: FadeCurve);
    pub fn tape_to(&mut self, ms: u32, kind: TapeKind, curve: FadeCurve);
    pub fn check_tape_completed(&mut self) -> bool;
    pub fn push(&mut self, input: &[i16]) -> Vec<[i16; FRAME_SAMPLES * CHANNELS]>;
}
```

**Key invariant**: `push()` accumulates input until a full `FRAME_SIZE` worth of `i16`
samples is available, then calls `process_frame()`. This guarantees the tape
Cubic Hermite interpolation window is always fed full frames.

---

## Phase 4 — Crossfade Controller

### 4.1  `audio/playback/crossfade.rs`  `[NEW]`
```rust
pub struct CrossfadeController {
    next_track_buf: RingBuffer,   // pre-buffered next-track PCM
    state: CrossfadeState,
    duration_ms: u32,
    elapsed_ms: f32,
    curve: FadeCurve,
    sample_rate: u32,
    channels: usize,
}

pub enum CrossfadeState { Idle, Buffering, Active }

impl CrossfadeController {
    pub fn prepare_next(&mut self, pcm: &[i16]);         // feed next-track PCM into ring
    pub fn is_ready(&self) -> bool;                       // enough buffered?
    pub fn start(&mut self, duration_ms: u32, curve: FadeCurve) -> bool;
    pub fn mix_frame(&mut self, main: &mut [i16]) -> bool; // returns false when done
    pub fn clear(&mut self);

    fn fade_gains(progress: f32, curve: FadeCurve) -> (f32, f32); // current, next
}
```

Constant-power curve (sinusoidal default):
```
gain_current = cos(progress * PI/2)
gain_next    = sin(progress * PI/2)
```

---

## Phase 5 — Mixer Audit

### 5.1  `audio/playback/mixer.rs` — audit existing
Current `Mixer` handles:
- `MixerTrack`: PCM `flume::Receiver<PooledBuffer>` + atomic state/volume/position.
- `PassthroughTrack`: raw Opus `flume::Receiver<Arc<Vec<u8>>>`.
- `mix(&mut [i16]) -> bool`: sums all active tracks.
- `take_opus_frame() -> Option<Arc<Vec<u8>>>`: polls passthrough before PCM path.

Ensure:
- Layer RingBuffer per mix-layer track (1 MB = ~5 s PCM).
- `has_active_layers() -> bool` for `FlowController` to check.
- `mix_layers_into(frame: &mut [i16])` reads from each layer ring, sums with per-layer gain.

---

## Phase 6 — HTTP / HLS Source

### 6.1  `audio/http_source/mod.rs` — audit existing
- `BaseHttpSource`: Range-HTTP reads with `prefetch_loop` thread ✓
- `SharedState` + `Condvar` pair for back-pressure ✓
- `content_type() -> Option<String>` for format detection ✓

### 6.2  `audio/http_source/hls.rs`  `[NEW]`
```rust
pub struct HlsReader { /* playlist_url, current_segment, ring */ }
impl HlsReader {
    pub fn new(url: &str, client: reqwest::Client) -> AnyResult<Self>;
    fn fetch_playlist(&mut self) -> AnyResult<Vec<SegmentInfo>>;
    fn fetch_segment(&mut self, seg: &SegmentInfo) -> AnyResult<Bytes>;
    fn decrypt_aes128(data: &[u8], key: &[u8; 16], iv: &[u8; 16]) -> Vec<u8>;
}
impl Read for HlsReader { ... }
impl MediaSource for HlsReader { ... }
```

---

## Phase 7 — Gateway Split

### 7.1  Split `gateway/session.rs` (969 lines, 3 concerns)

**`gateway/close_codes.rs`** `[NEW]`  
Extract the 3 close-code helpers and `SessionOutcome` enum:
```rust
pub enum SessionOutcome { Reconnect, Identify, Shutdown }
pub fn is_reconnectable_close(code: u16) -> bool { matches!(code, 1006 | 4015 | 4009) }
pub fn is_reidentify_close(code: u16) -> bool   { matches!(code, 4006) }
pub fn is_fatal_close(code: u16) -> bool         { matches!(code, 4004 | 4014) }
```

**`gateway/gateway.rs`** `[SPLIT]`  
Move `VoiceGateway`, `VoiceGatewayMessage`, `MAX_RECONNECT_ATTEMPTS`, `map_boxed_err()`,  
`VoiceGateway::new/run/run_session/discover_ip` from `session.rs`.

**`gateway/speak_loop.rs`** `[SPLIT]`  
Move `speak_loop()` (lines 836–969) from `session.rs`:
```rust
pub async fn speak_loop(
    mixer: Shared<Mixer>,
    socket: Arc<UdpSocket>,
    addr: SocketAddr,
    ssrc: u32,
    key: [u8; 32],
    mode: String,
    dave: Shared<DaveHandler>,
    filter_chain: Shared<FilterChain>,
    frames_sent: Arc<AtomicU64>,
    frames_nulled: Arc<AtomicU64>,
    cancel_token: CancellationToken,
) -> AnyResult<()>
```
Delete `gateway/session.rs` after both halves are extracted.

**Update `gateway/mod.rs`:** replace `pub mod session;` with:
```rust
pub mod close_codes;
pub mod gateway;
pub mod speak_loop;
pub use gateway::VoiceGateway;
```

---

## Phase 8 — Audio Processor Integration

### 8.1  `audio/processor.rs` — integrate FlowController

Current flow:
```
source → format_detect → decoder → resample → pcm_tx (channel)
```

Target flow:
```
source → format_detect
  ├── [passthrough] WebM+Opus, no active filters → opus_tx
  └── [transcode]  decode → resample → FlowController.push() → pcm_tx
```

`AudioProcessor::run_transcode()` feeds resampled i16 samples through
`FlowController.push()` and forwards processed frames into `pcm_tx`.

---

## Phase 9 — Player Wire-Up

### 9.1  `audio/playback/spawn.rs`  `[SPLIT from player/playback.rs]`
```rust
// Moved here because start_playback() directly constructs Mixer/TrackHandle.
// player/playback.rs is deleted after the split.
pub async fn start_playback(
    player: &mut PlayerContext,
    track: String,
    session: Arc<Session>,
    source_manager: Arc<SourceManager>,
    lyrics_manager: Arc<LyricsManager>,
    routeplanner: Option<Arc<dyn RoutePlanner>>,
    update_interval_secs: u64,
    user_data: Option<serde_json::Value>,
    end_time: Option<u64>,
)
```

Algorithm:
```
1. If track playing → emit TrackEnd(Replaced) → abort track_task → stop Mixer
2. Set player.track/track_info/position/paused/end_time/user_data/stop_signal
3. Resolve source: source_manager.get_track(&track_info, routeplanner)
4. Spawn lyrics fetch task (honours lyrics_subscribed flag)
5. source.start_decoding() → (pcm_rx, cmd_tx, error_rx, opus_rx)
6. TrackHandle::new(cmd_tx, tape_stop)
7. mixer.add_track(pcm_rx, audio_state, vol, pos, config)
8. if opus_rx.is_some() → mixer.add_passthrough_track(opus_rx, pos, state)
9. store handle in player.track_handle
10. emit TrackStart event
11. spawn monitor task (see player/monitor.rs) → store in player.track_task
```

### 9.2  `player/monitor.rs`  `[SPLIT from player/playback.rs]`
```rust
// The 500ms tick loop. Extracted from start_playback() into its own function.
pub async fn run_track_monitor(
    handle: TrackHandle,
    guild_id: GuildId,
    session: Arc<Session>,
    track_data: Track,
    stop_signal: Arc<AtomicBool>,
    ping: Arc<AtomicI64>,
    error_rx: flume::Receiver<String>,
    update_interval_secs: u64,
    stuck_threshold_ms: u64,
    lyrics_subscribed: Arc<AtomicBool>,
    lyrics_data: Arc<Mutex<Option<LyricsData>>>,
    last_lyric_index: Arc<AtomicI64>,
)
```

Responsibilities (500 ms tick):
- `stop_signal` check → exit
- `PlaybackState::Stopped` → check `error_rx` → emit `TrackException` or `TrackEnd(Finished)`
- Stuck detection: `current_pos == last_pos` for `stuck_threshold_ms` → `TrackStuck`
- `PlayerUpdate` every `update_interval_secs`
- Lyrics sync: position-based `LyricsLine` events (forward + backward seek handling)

### 9.3  `player/manager.rs`  `[NEW]`
```rust
pub struct PlayerMap(Arc<DashMap<GuildId, PlayerContext>>);
impl PlayerMap {
    pub fn get(&self, id: &GuildId) -> Option<...>;
    pub fn create(&self, id: GuildId, config: &PlayerConfig) -> ...;
    pub fn destroy(&self, id: &GuildId);
    pub fn count(&self) -> usize;
}
```

---

## Phase 10 — Verification

Run in order after each phase:

```bash
# 1. Compile check (fast, no tests)
cargo check

# 2. Lint
cargo clippy -- -D warnings

# 3. Unit tests (run per module)
cargo test audio::filters
cargo test audio::playback
cargo test player

# 4. Integration: full pipeline smoke test
cargo test --test integration

# 5. Format
cargo fmt --check
```

### Per-module test targets

| Module | What to test |
|--------|-------------|
| `audio::filters::dsp::biquad` | frequency response at cutoff |
| `audio::filters::dsp::lfo` | waveform period accuracy |
| `audio::filters::equalizer` | flat EQ = passthrough |
| `audio::playback::effects::tape` | ramp start/stop, `check_ramp_completed()` |
| `audio::playback::effects::fade` | gain at t=0, t=0.5, t=1 for each curve |
| `audio::playback::flow` | full frame accumulation, partial flush |
| `audio::playback::crossfade` | constant-power gains sum to ≈1.0 at all t |
| `audio::buffer` | pool alignment, GC via Drop |
| `audio::http_source::hls` | AES-128 decrypt round-trip |
| `gateway::close_codes` | all code ranges map to correct `SessionOutcome` |
| `player::monitor` | stuck counter resets on position change |
| `player::manager` | create/destroy/count |

---

## Phase Order Summary

```
0  - Constants + BufferPool audit
1  - DSP primitives reorganisation (filters/dsp/, FilterChain split, timescale/)
2  - Effects layer (tape audit, fade.rs, volume_ramp.rs, volume.rs expand)
3  - FlowController (flow.rs)
4  - CrossfadeController (crossfade.rs)
5  - Mixer audit (mix layers via RingBuffer)
6  - HLS reader (http_source/hls.rs)
7  - Gateway split (session.rs → gateway.rs + speak_loop.rs + close_codes.rs)
8  - AudioProcessor integration (use FlowController)
9  - Player wire-up (spawn.rs + monitor.rs + PlayerMap)
10 - Verify (cargo check, clippy, test, fmt)
```
