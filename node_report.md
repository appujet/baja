# NodeLink Audio Pipeline — How It Works

> A technical deep-dive into how audio flows inside **NodeLink-dev** (TypeScript/Node.js).

---

## Overview

```
Remote URL / Source
    │
    ▼
Format Detection (MIME / URL extension)
    │
    ├──▶ WebM/Opus  → WebmOpusDemuxer → raw Opus packets → OpusEncoder → Discord
    │
    ├──▶ AAC        → FAAD2NodeDecoder (WASM) → PCM f32 → Int16 → FlowController
    │
    ├──▶ MP4/M4A    → MP4Box → AAC stream → FAAD2 → PCM → FlowController
    │
    ├──▶ MPEG-TS    → AAC frame extractor → FAAD2 → PCM → FlowController
    │
    ├──▶ FLV        → FlvDemuxer → AAC frames → FAAD2 → PCM → FlowController
    │
    └──▶ Everything else → SymphoniaDecoder (WASM) → PCM → FlowController
                                                           │
                                                    FlowController
                                        ┌──────────────────┴───────────────────┐
                                    FiltersManager  TapeTransformer  VolumeTransformer
                                    FadeTransformer  AudioMixer
                                        └──────────────────┬───────────────────┘
                                                           │
                                                    OpusEncoder → CrossfadeController
                                                           │
                                                    Discord UDP (Opus encrypted)
```

All pipeline stages are Node.js `Transform` streams connected with `.pipe()`. Everything lives on the event loop — no Rust, no extra threads.

---

## 1. Format Detection (`streamProcessor.ts`)

`streamProcessor.ts` (2868 lines) is the core engine. On track start it reads the `Content-Type` header and URL extension and picks the right decoder path:

| Format detector | Condition |
|---|---|
| `_isWebmFormat()` | `content-type: audio/webm` or `.weba` |
| `_isFmp4Format()` | `fmp4`, `hls`, `mpegurl` in type |
| `_isMp4Format()` | `mp4`, `m4a`, `m4v`, `mov`, `quicktime` |
| `_isMpegtsFormat()` | `mpegts`, `video/mp2t` |
| `_isFlvFormat()` | `flv` in type |
| Fallback | `SymphoniaDecoder` (Rust WASM) — handles MP3, OGG, FLAC, WEBM… |

### Key constants

```typescript
AUDIO_CONFIG = { sampleRate: 48000, channels: 2, frameSize: 960, highWaterMark: 19200 }
BUFFER_THRESHOLDS = { maxCompressed: 256 KB, minCompressed: 128 KB }
AUDIO_CONSTANTS = { pcmFloatFactor: 32767, maxDecodesPerTick: 5, decodeIntervalMs: 10 }
```

---

## 2. Decoders

### 2a. `SymphoniaDecoderStream` — Universal fallback

Wraps `@toddynnn/symphonia-decoder` (Rust compiled to WASM):

- Receives compressed chunks via `_transform()`.
- Pushes them into the decoder's internal buffer.
- Back-pressure: if `bufferedBytes > 256 KB`, it stalls `callback()` until the decoder drains.
- Decode runs asynchronously via a `_decodeLoop()` with `setImmediate` yields between batches.
- Output: **PCM i16 stereo @ 48 kHz** (Buffer).

If source rate ≠ 48 kHz, `libsamplerate-js` (via WebAssembly) resamples. Quality levels: `best` (sinc), `medium`, `fastest`, `zero order holder`, `linear`.

### 2b. `FAAD2NodeDecoder` — AAC native decoder

- Used for AAC inside MP4, FMP4, MPEG-TS, and FLV.
- Frames are wrapped with a synthesized ADTS header (`_createAdtsHeader()`) before feeding FAAD2.
- Output: **PCM f32** → converted to i16 via `_floatToInt16Buffer()`.

### 2c. `WebmOpusDemuxer` — Raw Opus passthrough (no decode)

Parses the EBML/WebM container manually to extract raw Opus packets. No PCM decode — bytes go directly to the Opus encoder.

---

## 3. WebM/Opus Demuxer (`playback/demuxers/WebmOpus.ts`)

A streaming EBML parser built on top of `RingBuffer`:

### Design

```
HTTP stream → _transform() → ringBuffer.write(chunk)
                                  │
                                  └──▶ _readTag() loop (EBML tag scanner)
                                            │
                                  ┌─────────┴────────────┐
                             EBML Header             Audio Block (tag a3)
                             detected                     │
                                                 emit raw Opus packet bytes
```

### EBML VINT parsing

- `readVintLength()` — reads the leading set bit to get VINT width (1–8 bytes).
- `readVint()` — assembles the full bigint value from the VINT bytes.
- Unknown-size containers (EBML "unknown length") are handled by skipping.

### Key behaviour

| Tag | Action |
|---|---|
| `1a45dfa3` / `1f43b675` | EBML header found, enables tag scanning |
| `ae` | Start of a TrackEntry — reset pending track |
| `d7` | Track number |
| `83` | Track type (type 2 = audio) |
| `63a2` | Codec private data → emit `head` event (Opus header) |
| `a3` | Audio block → extract packet, `this.push(packet)` |

Uses a profiler tracking `packetsOut`, `bytesIn`, `ringPeakBytes`, etc.

---

## 4. HLS Handler (`playback/hls/`)

`HLSHandler.ts` handles adaptive HLS streams:

- `PlaylistParser.ts` — parses m3u8 manifests (variant + media playlists).
- `SegmentFetcher.ts` — fetches `.ts` segments with retry, reports download speeds.
- `AESDecryptor.ts` — AES-128 CBC decryption for encrypted HLS segments.
- Segments are fed into the MPEG-TS or AAC decoder once fetched.

---

## 5. FlowController (`playback/processing/FlowController.ts`)

The **central PCM processing hub**. All PCM data flows through here in exactly **3840-byte frames** (960 samples × 2 channels × 2 bytes = stereo i16).

### Per-frame processing order

```
incoming PCM chunk
    │ (reassembled into 3840-byte frames)
    ▼
FiltersManager.process()      ← EQ, reverb, karaoke…
    │
    ▼
TapeTransformer.process()     ← tape stop/start speed ramp
    │
    ▼
VolumeTransformer.process()   ← gain + AGC + soft limiter
    │
    ▼
FadeTransformer.process()     ← crossfade fade-in/out gain
    │
    ▼
AudioMixer.mixBuffers()       ← optional layer overlay mixing
    │
    ▼
this.push(output)             ← to OpusEncoder / CrossfadeController
```

Incoming chunks that don't align to 3840 bytes are accumulated in `pendingBuffer` until a full frame is available.

---

## 6. TapeTransformer (`playback/processing/TapeTransformer.ts`)

Simulates cassette tape slowing/speeding using **Cubic Hermite Spline interpolation**.

### How it works

```
tapeTo(durationMs, 'stop', 'sinusoidal')
    ↓
tape = { startRate: 1.0, targetRate: 0.01, durationMs, elapsedMs: 0 }

Per frame:
    elapsedMs += sampleDurationMs
    t = elapsedMs / durationMs               (0.0 → 1.0)
    curveT = (1 - cos(t * π)) / 2            (sinusoidal)
    currentRate = startRate + (targetRate - startRate) * curveT

    // Cubic Hermite read at fractional position
    readPos += currentRate * channels        // < 1.0 means slowing down
    val = hermite(p0, p1, p2, p3, frac)
```

### Interpolation formula (Catmull-Rom Hermite)

```
val = 0.5 × (2p1 + (-p0+p2)×frac + (2p0-5p1+4p2-p3)×frac² + (-p0+3p1-3p2+p3)×frac³)
```

This gives smoother pitch shift than linear interpolation (baja uses linear; NodeLink uses cubic).

### Curves supported

| Curve | Formula |
|---|---|
| `linear` | `t` |
| `exponential` | `t²` |
| `sinusoidal` | `(1 - cos(t×π)) / 2` (default) |

### Buffer management

- Maintains a `Float32Array` sliding window (10 seconds, `48000 * 2 * 10` samples).
- `_compact()` shifts unread data to the front when `readPos > 2s` worth of samples.

---

## 7. VolumeTransformer (`playback/processing/VolumeTransformer.ts`)

Handles volume gain, fade curves, lookahead peak limiting, and optional AGC.

### Features

| Feature | Detail |
|---|---|
| **Volume gain** | Multiplies each i16 by current gain |
| **Fade curves** | `linear`, `sine`/`sinusoidal` — computed as linear interpolation between gainStart/gainEnd per frame |
| **Lookahead buffer** | 5 ms (240 samples) ring — delays output by 5 ms, enabling peak detection before output |
| **Soft limiter** | Exponential knee at threshold (default 95%). `limited = threshold + headroom × (1 - e^(-overshoot × softness))` |
| **AGC (LoudnessNormalizer)** | Targets -14 LUFS; gated below configurable threshold |

### Processing loop (fast path, no lookahead)

```
gainStep = (gainEnd - gainStart) / sampleCount
for each sample:
    output = clamp(applyLimiter(sample × gain))
    gain += gainStep
```

---

## 8. AudioMixer (`playback/processing/AudioMixer.ts`)

Overlays multiple PCM **layers** onto the main track. Used for sound effects, notifications over music, etc.

### Layer lifecycle

```
addLayer(stream, track, volume)
    │
    ├── Attach 'data' handler → ringBuffer.write(chunk)
    ├── Auto-pause stream when ringBuffer > 80% full
    └── Auto-resume when < 50% full

readLayerChunks(chunkSize)
    └── Read from each layer's RingBuffer → Map<id, {buffer, volume}>

mixBuffers(mainPCM, layersPCM)
    └── sample[i] = mainView[i] + Σ (layerView[i] × layerVolume)
        clamped to [-32768, 32767]
```

### Key limits

- Max layers: 5 (configurable)
- Each layer RingBuffer: **1 MB** (~5 seconds of PCM)
- Default layer volume: 0.8

---

## 9. CrossfadeController (`playback/processing/CrossfadeController.ts`)

Smoothly transitions from one track to the next.

### Three-phase flow

```
Phase 1 — Prepare:
    prepareNextStream(pcmStream, { durationMs: 5000 })
    → Opens next track's PCM stream
    → Buffers into RingBuffer (up to durationMs worth of PCM)
    → Pauses next stream when buffer full

Phase 2 — Ready check:
    isReady() → ringBuffer.length >= minBufferBytes

Phase 3 — Crossfade:
    startCrossfade(5000, 'sinusoidal')
    → _transform() called per main chunk:
         nextChunk = ringBuffer.read(data.length)
         output = _mixBuffers(main, next, runtime)
```

### `_mixBuffers` — constant-power fade

```
progress = elapsedMs / durationMs

// sinusoidal (default):
fadeOut = cos(progress × π/2)    // main track fades out
fadeIn  = sin(progress × π/2)    // next track fades in

// linear:
fadeOut = 1 - progress
fadeIn  = progress

sample = mainSample × fadeOut + nextSample × fadeIn
```

Constant-power curve prevents the loudness dip that linear fades cause.

---

## 10. RingBuffer (`playback/structs/RingBuffer.ts`)

Fixed-size circular buffer backed by pooled memory.

```
[  readOffset ───────────── writeOffset  ]
 ↑                                       ↑
 read from here              write continues here (wraps)
```

- `write(chunk)` — wraps around at end, overwrites oldest if full.
- `read(n)` — copies n bytes out (via BufferPool), advances readOffset.
- `skip(n)` — advances readOffset without copy.
- `peek(n)` — reads without advancing.
- `getContiguous(n)` — returns a zero-copy subarray if data is contiguous.
- `dispose()` — returns internal buffer to pool.

---

## 11. BufferPool (`playback/structs/BufferPool.ts`)

Global singleton for reusing `Buffer` allocations.

### Key design

| Detail | Value |
|---|---|
| Alignment | Sizes rounded up to next power-of-two (min 1024) |
| Max pool memory | 50 MB (env `NODELINK_BUFFER_POOL_MAX_BYTES`) |
| Max entries per bucket | 8 (env `NODELINK_BUFFER_POOL_MAX_BUCKET_ENTRIES`) |
| Poolable range | 1 KB – 10 MB |
| Idle clear | 180 s of no activity (env `NODELINK_BUFFER_POOL_IDLE_CLEAR_MS`) |
| Cleanup interval | Every 60 s |

### Stats tracked

`acquireCalls`, `reuseHits`, `newAllocs`, `reuseRatio`, `highWaterBytes`, per-bucket breakdown — all accessible via `getStats()`.

---

## 12. Opus Encoder/Decoder (`playback/opus/Opus.ts`)

Wraps `audiopus` (or similar native binding):

- **`Encoder`**: consumes PCM i16 stereo at 48 kHz, produces Opus frames. Frame size: 960 samples (20 ms).
- **`Decoder`**: for Opus→PCM when filters need to process a passthrough stream.

---

## 13. Filters (`playback/filters/`)

20+ DSP filters, all operating on **PCM i16 buffers** in-place:

| Category | Filters |
|---|---|
| EQ / Frequency | `equalizer`, `highpass`, `lowpass` |
| Dynamics | `compressor` |
| Spatial | `karaoke`, `channelMix`, `rotation`, `spatial` |
| Modulation | `chorus`, `flanger`, `phaser`, `vibrato`, `tremolo` |
| Reverb / Echo | `reverb`, `echo` |
| Creative | `distortion`, `phonograph` |
| Time / Pitch | `timescale` (+ `dsp/` submodule) |
| Base | `BaseFilter.ts` — interface all filters implement |

All registered and applied through `FiltersManager` (`processing/filtersManager.ts`).

---

## 14. End-to-End: What Happens When a Track Plays

```
1. source (YouTube, Spotify, etc.) resolves a stream URL
2. streamProcessor detects format from Content-Type / URL extension
3a. WebM/Opus → WebmOpusDemuxer → raw packets → OpusEncoder → Discord
3b. Other → decoder (Symphonia/FAAD2/FLV) → PCM i16 @ 48kHz
4. PCM enters FlowController:
      └─ FiltersManager → TapeTransformer → VolumeTransformer
         → FadeTransformer → AudioMixer → push()
5. Output goes to CrossfadeController (if crossfade active: blend with next track)
6. Final PCM → OpusEncoder (960 samples = 20ms frame)
7. Encrypted Opus frame → @performanc/voice → Discord UDP
```

---

## Key Architectural Differences vs baja (Rust)

| Aspect | NodeLink (TS) | baja (Rust) |
|---|---|---|
| Language | TypeScript / Node.js | Rust |
| Concurrency model | Event loop + streams | OS threads + flume channels |
| Decoders | Symphonia WASM, FAAD2 WASM | Symphonia native, audiopus |
| Tape interpolation | **Cubic Hermite spline** | **Linear interpolation** |
| Volume fade | Sinusoidal + limiter + AGC | Sinusoidal (no AGC/limiter) |
| Crossfade | `CrossfadeController` (constant-power cos/sin) | Not implemented |
| Multi-layer mixing | `AudioMixer` (up to 5 layers, RingBuffer per layer) | Single track in `Mixer` |
| Buffer pool | Power-of-2 aligned, 50 MB cap, per-bucket | Fixed 4096-sample Vec pool |
| Ring buffer | Pointer-based circular, pool-backed | N/A (uses flume channel) |
| Tape effect | Cubic + `linear`/`exponential`/`sinusoidal` curves | Linear only |
| HLS | `HLSHandler` + `PlaylistParser` + AES decrypt | Separate `hls/` module |
| Format detection | MIME string matching | Symphonia probe |
