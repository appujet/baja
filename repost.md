# Audio Pipeline — How It Works

> A technical deep-dive into how audio flows from a remote URL to Discord voice packets in **baja**.

---

## Overview

```
Remote URL
    │
    ▼
BaseRemoteReader          ← prefetch thread, 8 MB ring buffer, seek-optimized
    │
    ▼ MediaSource (Symphonia)
AudioProcessor            ← probe → demux → decode → resample
    │               │
    │   [Opus passthrough]   raw Opus frames (Arc<Vec<u8>>)
    │               │
    │   [PCM transcode]      PooledBuffer (i16 stereo @ 48 kHz)
    │               │
    ▼               ▼
        Mixer
    │
    ▼
gateway/session speak_loop
    │
    ▼
Discord UDP (Opus encrypted)
```

Every piece is connected by **flume** lock-free channels. No shared mutable state crosses thread boundaries — only atomics and message passing.

---

## 1. Remote Reader (`audio/remote_reader/`)

**File:** `mod.rs` + `segmented.rs`

`BaseRemoteReader` is the HTTP source that feeds everything. It implements `Read`, `Seek`, and Symphonia's `MediaSource`.

### How it fetches

- On creation it immediately fires a `Range: bytes=0-` request — no wasted round-trip.
- A dedicated **prefetch background thread** (`remote-prefetch`) continuously downloads and fills a `next_buf`.
- The thread caps the in-memory buffer at **8 MB**. When that limit is hit it parks on a `Condvar` to avoid unbounded memory growth.

### Seek strategy (three tiers)

| Situation | Action | Latency |
|---|---|---|
| Jump lands inside already-downloaded bytes | Pure in-memory pointer advance | 0 ms |
| Small forward jump (≤ 1 MB) | Socket-skip (drain chunks over the live TCP conn) | ~0 ms (no TCP teardown) |
| Large or backward jump | Hard re-connect with new `Range` request | ~300 ms |

### `SegmentedRemoteReader`

Used by sources that expose audio as numbered segments (e.g. YouTube HLS/DASH). It stitches segments into a single seekable stream, fetching the next segment just before the current one ends.

---

## 2. Audio Processor (`audio/processor.rs`)

`AudioProcessor` wraps a `MediaSource` and runs the full decode pipeline on a dedicated thread.

### Initialization

```
MediaSource
  │  MediaSourceStream
  ▼
Symphonia probe()       ← reads container header, picks format reader
  │
  ▼
find first non-null track
  │
  ├── CODEC_TYPE_OPUS + opus_tx provided → Passthrough mode (no decoder created)
  ├── CODEC_TYPE_OPUS + no opus_tx → Transcode mode (custom OpusCodecDecoder)
  └── Any other codec              → Transcode mode (Symphonia built-in decoder)
```

A `Hint` is passed from the source (e.g. `"webm"`, `"mp4"`) so Symphonia skips probing ambiguity.

### Passthrough mode (`run_passthrough`)

For WebM/Opus streams with **no active filters**:

1. Read raw packet bytes from container.
2. Wrap in `Arc<Vec<u8>>`.
3. Send over `opus_tx` channel.

**Zero decode. Zero resample. Zero encode.** The bytes go straight to Discord as-is.

### Transcode mode (`run_transcode`)

For all other codecs (MP3, AAC, FLAC, etc.) or when filters are active:

1. `format.next_packet()` — demux one packet.
2. `decoder.decode(&packet)` — decode to `AudioBuffer<f32>`.
3. `buf.copy_interleaved_ref(decoded)` — convert to `SampleBuffer<i16>`.
4. If `source_rate != 48000` → `Resampler::process()`.
5. `pcm_tx.send(pooled)` — push `PooledBuffer` to Mixer.

### Commands

The processor checks `cmd_rx` at the top of every loop iteration:

| Command | Effect |
|---|---|
| `Seek(ms)` | `format.seek()` + `resampler.reset()` + `decoder.reset()` |
| `Stop` | Break the loop immediately |

---

## 3. Resampler (`audio/pipeline/resampler.rs`)

A lightweight **linear interpolation resampler** — no external DSP crates.

```
ratio = source_rate / target_rate   (e.g. 44100/48000 = 0.918...)

for each output sample:
    idx   = floor(index)
    fract = index - idx
    out   = s[idx-1] * (1 - fract) + s[idx] * fract
    index += ratio
```

State (`index`, `last_samples`) persists across calls so frame boundaries are seamless. `reset()` zeroes state after a seek.

---

## 4. Buffer Pool (`audio/buffer.rs`)

`PooledBuffer` is a **recycled `Vec<i16>`** — avoids a heap allocation every 20 ms frame.

- Global singleton: `GLOBAL_BUFFER_POOL` (`OnceLock`).
- Pool holds up to **128 buffers**, each pre-allocated to 4096 samples.
- `acquire()` pops from pool (or allocates fresh if empty).
- `Drop` implementation returns the buffer to the pool automatically.

This eliminates ~50 allocations/second at 48 kHz stereo.

---

## 5. Mixer (`audio/playback/mixer.rs`)

The `Mixer` runs inside the **speak loop** (gateway thread). It is called once every 20 ms to produce one 960-sample stereo frame (1920 i16 samples).

### Data structures

```rust
Mixer {
    tracks: Vec<MixerTrack>,       // PCM tracks (transcode path)
    mix_buf: Vec<i32>,             // i32 accumulator (headroom for mixing)
    opus_passthrough: Option<...>, // raw Opus shortcut
}
```

### Per-tick flow

```
1. Drain stopped tracks from `tracks` vec.
2. Poll opus_passthrough first:
     → if frame available: return raw bytes directly (skip all PCM work).
3. For each PCM track:
     a. Read PlaybackState (atomic, no lock).
     b. If Paused/Stopped → skip.
     c. If Stopping/Starting → run TapeEffect.
     d. Otherwise → normal mix:
          i.  Drain leftover samples from previous frame (pending buf).
          ii. Receive new PooledBuffers from channel (try_recv, non-blocking).
         iii. Accumulate into mix_buf with fixed-point volume scaling.
4. Clamp i32 mix_buf → i16 output (saturation, no wrap-around distortion).
```

### Volume scaling (fixed-point, no floats in the hot path)

```rust
vol_fixed = (vol * 65536.0) as i32;
sample_out = (sample * vol_fixed) >> 16;
```

At `vol = 1.0` (`vol_fixed = 65536`), the branch skips the multiply entirely.

---

## 6. Playback States & TrackHandle (`audio/playback/handle.rs`)

```
Playing  ──pause()──▶  Stopping ──TapeEffect done──▶ Paused
Paused   ──play()───▶  Starting ──TapeEffect done──▶ Playing
Any      ──stop()───▶  Stopped
```

`TrackHandle` is the **external control surface** — cloned freely and used from the HTTP handler thread:

| Method | Effect |
|---|---|
| `pause()` | → `Stopping` (if tape_stop enabled) or `Paused` |
| `play()` | → `Starting` (if tape_stop enabled) or `Playing` |
| `stop()` | → `Stopped` (SeqCst, immediately visible) |
| `set_volume(f32)` | Stores f32-as-bits into atomic (lock-free) |
| `seek(ms)` | Updates position atomic + sends `DecoderCommand::Seek` |
| `get_position()` | `samples * 1000 / 48000` → milliseconds |

All state is stored in **atomics** (`AtomicU8`, `AtomicU32`, `AtomicU64`). The speak loop reads them without a mutex — zero contention.

---

## 7. Tape Stop Effect (`audio/playback/effects/tape.rs`)

Simulates a cassette tape slowing down (pause) or spinning up (resume).

### Algorithm

```
duration_ms → frames = duration_ms * 48.0  (48kHz stereo frames per ms)
step = 1.0 / frames

Stopping: rate decreases by step each stereo sample pair
          rate hits 0.0 → set state = Paused

Starting: rate increases by step each stereo sample pair
          rate hits 1.0 → set state = Playing
```

### Read position (variable speed)

```rust
read_idx = floor(pos)
frac     = pos - read_idx

// Linear interpolation per channel
out = s[read_idx] + (s[read_idx+1] - s[read_idx]) * frac

pos += rate   // rate < 1.0 means slowing down = reading fewer source samples
```

The stash buffer (`pending`) is refilled from `rx` on-demand. Position tracking accumulates `samples_consumed` (a float) to handle sub-sample steps.

---

## 8. Codec: Opus Decoder (`audio/codecs/opus.rs`)

When Opus needs to be decoded to PCM (filters active), a custom `OpusCodecDecoder` wraps `audiopus`:

- Implements Symphonia's `Decoder` trait.
- Decodes into a `i16` buffer at 48 kHz stereo.
- Handles packet loss and DTX (discontinuous transmission) gracefully.

---

## 9. Filters (`audio/filters/`)

24 audio filters are available. They all operate on the **PCM path** (transcode mode only — passthrough skips them all).

| Category | Filters |
|---|---|
| EQ / Frequency | `equalizer`, `high_pass`, `low_pass`, `biquad` |
| Dynamics | `compressor`, `normalization`, `volume` |
| Spatial | `karaoke`, `channel_mix`, `rotation`, `spatial` |
| Modulation | `chorus`, `flanger`, `phaser`, `vibrato`, `tremolo` |
| Time / Pitch | `timescale` |
| Reverb / Delay | `reverb`, `echo`, `delay_line` |
| Creative | `distortion`, `phonograph` |
| Modulation helper | `lfo` |

Filters are applied **per-frame** inside the processor decode loop before samples are sent to the mixer.

---

## 10. End-to-End: What Happens When You Play a Track

```
1. HTTP handler receives /v4/sessions/{id}/players/{guild_id} PATCH
2. PlayerPlayback::start_track() called
3. Source (e.g. YouTubeSource) resolves URL → returns Box<dyn MediaSource>
4. BaseRemoteReader wraps the URL, starts prefetch thread
5. AudioProcessor::new_with_passthrough() probes the stream:
     - WebM/Opus + no filters → passthrough mode
     - Otherwise              → transcode mode
6. Processor thread spawned, starts run() loop
7. TrackHandle registered with Mixer (add_track / add_passthrough_track)
8. speak_loop (gateway thread) calls Mixer::mix() every 20 ms:
     - Passthrough: reads Arc<Vec<u8>> from opus_passthrough channel
     - PCM: reads PooledBuffer, mixes with volume, clamps to i16
9. Encrypted Opus frame → UDP → Discord voice server
10. Position updated via AtomicU64 after each frame
```

---

## Key Design Decisions

| Decision | Rationale |
|---|---|
| Flume channels everywhere | Lock-free, no Mutex on hot path |
| AtomicU8 for playback state | Readable from speak loop and HTTP thread simultaneously |
| `Arc<Vec<u8>>` for Opus passthrough | Zero-copy — same bytes sent to Discord as received from CDN |
| PooledBuffer (recycled Vec) | Eliminates ~50 heap allocs/sec at 48 kHz |
| 8 MB prefetch cap with Condvar park | Bounded memory, no busy-wait |
| Socket-skip for small seeks | Avoids TCP teardown latency on small forward seeks |
| Fixed-point volume multiply | No float in the mixer inner loop |
