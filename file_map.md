# baja — Rust File Structure Map

> Rust-idiomatic layout. Each file owns one concern. No JS patterns.
> `mod.rs` only used for re-exports, never for logic.

---

## Design Rules

- One struct / trait / enum per file where practical.
- `mod.rs` contains only `pub use` re-exports and `pub mod` declarations.
- Shared primitives (error types, small types, traits) live in `common/`.
- Config structs are kept in `configs/` and never carry logic.
- Audio pipeline logic is split between `audio/` (DSP, decoding) and `player/` (lifecycle, state).
- Each audio filter is its own file under `audio/filters/`.
- DSP primitives used across multiple filters are in `audio/filters/dsp/`.

---

## Current Layout vs. Target

```
src/
├── main.rs                           # Entry point, runtime setup
├── lib.rs                            # Crate-level re-exports (kept minimal)
│
├── common/                           # Shared types, traits, utilities
│   ├── mod.rs
│   ├── errors.rs                     # AppError, AnyResult, thiserror impls
│   ├── types.rs                      # GuildId, AudioKind, Shared<T>, small enums
│   ├── http.rs                       # reqwest client builder helper
│   └── logger/
│       ├── mod.rs
│       ├── filter.rs                 # Log level filter
│       └── formatter.rs             # Log formatter
│
├── configs/                          # Pure config structs, no logic
│   ├── mod.rs
│   ├── base.rs                       # Top-level ServerConfig
│   ├── server.rs                     # HTTP / WS bind config
│   ├── player.rs                     # PlayerConfig (tape_stop_duration_ms, etc.)
│   ├── filters.rs                    # FiltersConfig defaults
│   ├── sources.rs                    # Per-source enable/disable/auth config
│   └── lyrics.rs                     # LyricsConfig
│
├── audio/                            # All DSP, decode, encode, streaming
│   ├── mod.rs
│   ├── constants.rs             [NEW] # FRAME_SIZE, SAMPLE_RATE, CHANNELS, etc.
│   ├── buffer.rs                     # PooledBuffer, BufferPool (power-of-2 buckets)
│   │
│   ├── processor.rs                  # AudioProcessor (passthrough / transcode loop)
│   │                                 #   DecoderCommand enum
│   │                                 #   CommandOutcome enum
│   │
│   ├── http_source/                  # HTTP source reading
│   │   ├── mod.rs                    # BaseHttpSource (Read+Seek+MediaSource)
│   │   │                             #   prefetch_loop, SharedState, PrefetchCommand
│   │   ├── segmented.rs              # SegmentedReader for YouTube-style DASH segments
│   │   └── hls.rs               [NEW] # HLS reader: playlist parse, AES decrypt, TS fetch
│   │
│   ├── codecs/                       # Encode / decode wrappers
│   │   ├── mod.rs
│   │   └── opus.rs                   # OpusEncoder, OpusDecoder (via audiopus)
│   │
│   ├── pipeline/                     # PCM → Opus output path
│   │   ├── mod.rs
│   │   ├── encoder.rs                # PCM i16 → Opus packet
│   │   └── resampler.rs              # Any-rate → 48 kHz via libsamplerate
│   │
│   ├── filters/                      # All DSP effect filters
│   │   ├── mod.rs                    # FilterChain, AudioFilter trait, FilterKind enum
│   │   │                             # filter_chain_from_config()
│   │   ├── dsp/                 [NEW]  # Shared DSP primitives (no public API)
│   │   │   ├── mod.rs
│   │   │   ├── biquad.rs             # moved from filters/biquad.rs
│   │   │   ├── lfo.rs                # moved from filters/lfo.rs
│   │   │   ├── delay_line.rs         # moved from filters/delay_line.rs
│   │   │   └── clamp.rs         [NEW] # i16/f32 clamp helpers
│   │   ├── equalizer.rs
│   │   ├── karaoke.rs
│   │   ├── timescale/           [NEW]  # Timescale as its own submodule
│   │   │   ├── mod.rs                # Timescale filter (re-exports)
│   │   │   ├── time_stretch.rs  [NEW] # WSOLA / overlap-add stretching
│   │   │   └── resampler.rs     [NEW] # Rate conversion for timescale
│   │   ├── tremolo.rs
│   │   ├── vibrato.rs
│   │   ├── rotation.rs
│   │   ├── distortion.rs
│   │   ├── channel_mix.rs
│   │   ├── low_pass.rs
│   │   ├── high_pass.rs
│   │   ├── echo.rs
│   │   ├── chorus.rs
│   │   ├── compressor.rs
│   │   ├── flanger.rs
│   │   ├── phaser.rs
│   │   ├── phonograph.rs
│   │   ├── reverb.rs
│   │   ├── spatial.rs
│   │   ├── normalization.rs          # LoudnessNormalizer / AGC
│   │   └── volume.rs                 # VolumeFilter (per-sample gain + fade curves)
│   │
│   └── playback/                     # PCM-frame mixing and output
│       ├── mod.rs                    # re-exports: Mixer, TrackHandle, PlaybackState
│       ├── handle.rs                 # TrackHandle, PlaybackState enum
│       │                             # (pause/play/stop/seek/set_volume, atomic state)
│       ├── mixer.rs                  # Mixer: multi-track PCM mix + Opus passthrough
│       │                             # MixerTrack, PassthroughTrack
│       ├── spawn.rs             [SPLIT from player/playback.rs]
│       │                             # start_playback(): resolve source → create pipeline
│       │                             # → add_track/add_passthrough_track → emit TrackStart
│       ├── crossfade.rs         [NEW] # CrossfadeController: pre-buffers next-track PCM
│       │                             # constant-power curve mix during fade window
│       ├── flow.rs              [NEW] # FlowController: ordered DSP frame pipeline
│       │                             # filters → tape → volume → fade → mixer
│       └── effects/
│           ├── mod.rs                # TransitionEffect trait
│           ├── tape.rs               # TapeEffect (TapeState enum: Stopping/Starting)
│           ├── fade.rs          [NEW] # FadeTransformer: scheduled envelope (fade-in/out)
│           └── volume_ramp.rs   [NEW] # Sinusoidal / linear ramp for volume transitions
│
├── player/                           # Player lifecycle, state machine
│   ├── mod.rs
│   ├── context.rs                    # PlayerContext (data-only struct, no logic)
│   ├── state.rs                      # PlayerState, VoiceConnectionState, Filters, Player
│   ├── monitor.rs               [SPLIT from player/playback.rs]
│   │                                 # track_task loop: PlayerUpdate timer, stuck detection,
│   │                                 # lyrics line sync, error → TrackException/TrackEnd
│   └── manager.rs               [NEW] # PlayerMap: Arc<DashMap<GuildId, PlayerContext>>
│                                     # create_player, destroy_player, count
│
├── sources/                          # Music source providers
│   ├── mod.rs
│   ├── manager.rs                    # SourceManager: resolve, load_track, search
│   ├── plugin.rs                     # PluginSource trait
│   ├── http/                         # Direct HTTP stream source
│   │   ├── mod.rs
│   │   └── resolver.rs
│   ├── local/                        # Local file source
│   │   └── mod.rs
│   ├── youtube/                      # YouTube + YTMusic (34 files, keep as-is)
│   ├── spotify/
│   ├── deezer/
│   ├── tidal/
│   ├── soundcloud/
│   ├── jiosaavn/
│   ├── gaana/
│   ├── applemusic/
│   ├── bandcamp/
│   ├── audiomack/
│   ├── audius/
│   ├── mixcloud/
│   ├── anghami/
│   ├── pandora/
│   ├── qobuz/
│   ├── shazam/
│   └── yandexmusic/
│
├── gateway/                          # Discord voice gateway
│   ├── mod.rs                        # pub use re-exports
│   ├── engine.rs                     # VoiceEngine: mixer Arc, connection state
│   ├── encryption.rs                 # XSalsa20-Poly1305 / AES-256-GCM-RTP packet encryption
│   ├── udp_link.rs                   # UdpBackend: RTP framing, sequence counter, UDP send
│   ├── close_codes.rs           [NEW] # is_reconnectable_close(), is_reidentify_close(),
│   │                                 # is_fatal_close(), SessionOutcome enum
│   ├── gateway.rs               [SPLIT from session.rs]
│   │                                 # VoiceGateway struct + run() + run_session() + discover_ip()
│   │                                 # VoiceGatewayMessage, MAX_RECONNECT_ATTEMPTS
│   └── speak_loop.rs            [SPLIT from session.rs]
│                                     # speak_loop(): 20 ms tick, Opus passthrough,
│                                     # PCM encode, DAVE encrypt, UDP send
│
├── transport/                        # HTTP + WebSocket server
│   ├── mod.rs
│   ├── http_server.rs
│   ├── websocket_server.rs
│   └── routes/                       # Axum route handlers
│       ├── mod.rs
│       └── ...                       # (keep existing 10 files)
│
├── api/                              # Lavalink protocol types
│   ├── mod.rs
│   ├── events.rs                     # PlayerEvent, TrackEvent enums
│   ├── models.rs                     # Track, SearchResult, LyricsData
│   ├── opcodes.rs                    # WS opcode definitions
│   ├── tracks.rs                     # Track search/resolve response shapes
│   ├── info.rs                       # Server info response
│   ├── stats.rs                      # Stats response
│   ├── session.rs                    # Session model
│   └── routeplanner.rs               # RoutePlanner API types
│
├── lyrics/                           # Lyrics fetchers
│   └── ...                           # (keep existing structure)
│
├── monitoring/                       # Metrics / profiling
│   └── ...
│
├── routeplanner/                     # IP rotation
│   └── ...
│
└── server/                           # Shared server state (Session, etc.)
    └── ...
```

---

## New Files to Create  `[NEW]`

| File | Why |
|------|-----|
| `audio/constants.rs` | Single source of truth for `FRAME_SIZE=3840`, `SAMPLE_RATE=48_000`, `CHANNELS=2`, buffer thresholds |
| `audio/http_source/hls.rs` | HLS playlist parsing, segment fetching, AES-128 decryption — currently absent |
| `audio/filters/dsp/` (subdir) | Extract `biquad.rs`, `lfo.rs`, `delay_line.rs` into a `dsp/` submodule so filters only import what they need |
| `audio/filters/dsp/clamp.rs` | `clamp_i16()`, `clamp_f32()` — currently duplicated inline across filters |
| `audio/filters/timescale/` (subdir) | `time_stretch.rs` + `resampler.rs` — split timescale.rs (~4.6 KB) to isolate WSOLA from rate math |
| `audio/playback/spawn.rs` | `start_playback()` moved here from `player/playback.rs`; near Mixer/TrackHandle where it belongs |
| `audio/playback/crossfade.rs` | `CrossfadeController`: pre-buffers next-track PCM, constant-power mix |
| `audio/playback/flow.rs` | `FlowController`: ordered frame path — filters→tape→volume→fade→mixer |
| `audio/playback/effects/fade.rs` | `FadeTransformer`: scheduled fade-in/out envelopes |
| `audio/playback/effects/volume_ramp.rs` | Sinusoidal / linear / exponential gain ramp |
| `player/monitor.rs` | `track_task` loop: PlayerUpdate timer, stuck detection, lyrics sync, error handling |
| `player/manager.rs` | `PlayerMap` wrapper around `DashMap<GuildId, PlayerContext>` |

---

## Files to Reorganise / Split

| Current path | Target path | Reason |
|---|---|---|
| `gateway/session.rs` | split → `gateway/gateway.rs` + `gateway/speak_loop.rs` + `gateway/close_codes.rs` | 969-line file with 3 unrelated concerns: WS state machine, speak loop, close code helpers |
| `player/playback.rs` | split → `audio/playback/spawn.rs` + `player/monitor.rs` | `start_playback()` belongs near Mixer; the 500ms monitor loop + lyrics sync is its own concern |
| `audio/filters/biquad.rs` | `audio/filters/dsp/biquad.rs` | DSP primitive, not a user-facing filter |
| `audio/filters/lfo.rs` | `audio/filters/dsp/lfo.rs` | Shared LFO primitive |
| `audio/filters/delay_line.rs` | `audio/filters/dsp/delay_line.rs` | Shared delay primitive |
| `audio/filters/volume.rs` | expand in-place | Add fade-curve logic (currently only 641 B, too thin) |
| `audio/filters/mod.rs` | split → `mod.rs` + `audio/filters/chain.rs` | `mod.rs` is 16 KB — move `FilterChain` impl to `chain.rs` |

---

## Key Types Index

| Type | File | Role |
|------|------|------|
| `AudioProcessor` | `audio/processor.rs` | Decode loop, passthrough/transcode mode selection |
| `DecoderCommand` | `audio/processor.rs` | Seek, Stop (sent via `flume` channel) |
| `Mixer` | `audio/playback/mixer.rs` | Multi-track PCM summation + Opus passthrough |
| `MixerTrack` | `audio/playback/mixer.rs` | Per-track receiver + volume + state |
| `PassthroughTrack` | `audio/playback/mixer.rs` | Raw Opus frame receiver |
| `TrackHandle` | `audio/playback/handle.rs` | Atomic state/volume/position + command sender |
| `PlaybackState` | `audio/playback/handle.rs` | `Playing/Paused/Stopped/Stopping/Starting` |
| `start_playback()` | `audio/playback/spawn.rs` | Resolve source → init pipeline → emit TrackStart |
| `FlowController` | `audio/playback/flow.rs` | Ordered DSP frame path (to be created) |
| `CrossfadeController` | `audio/playback/crossfade.rs` | Next-track pre-buffer + constant-power mix |
| `TapeEffect` | `audio/playback/effects/tape.rs` | Cubic-Hermite speed ramp on pause/resume |
| `FadeTransformer` | `audio/playback/effects/fade.rs` | Scheduled fade-in/out envelope |
| `FilterChain` | `audio/filters/chain.rs` | Sorted filter list, `process(&mut [i16])` |
| `AudioFilter` | `audio/filters/mod.rs` | Trait: `process()`, `priority()`, `reset()` |
| `BufferPool` | `audio/buffer.rs` | Power-of-two bucket pool (singleton) |
| `PooledBuffer` | `audio/buffer.rs` | RAII guard that returns buffer to pool on drop |
| `BaseHttpSource` | `audio/http_source/mod.rs` | Seekable HTTP source with prefetch thread |
| `SegmentedReader` | `audio/http_source/segmented.rs` | DASH/YouTube segment reader |
| `VoiceGateway` | `gateway/gateway.rs` | WS connect, Identify/Resume, op dispatch, reconnect loop |
| `speak_loop()` | `gateway/speak_loop.rs` | 20 ms RTP tick: Opus passthrough / PCM encode / DAVE encrypt / UDP send |
| `SessionOutcome` | `gateway/close_codes.rs` | `Reconnect / Identify / Shutdown` enum + close-code helpers |
| `PlayerContext` | `player/context.rs` | All per-player state (data-only struct, no logic) |
| `PlayerState` | `player/state.rs` | Serialisable player snapshot |
| `track_task loop` | `player/monitor.rs` | 500 ms tick: stuck detection, PlayerUpdate, lyrics sync |
| `PlayerMap` | `player/manager.rs` | `DashMap<GuildId, PlayerContext>` CRUD wrapper |
