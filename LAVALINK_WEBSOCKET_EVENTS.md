# Lavalink v4 WebSocket Events — Full Comparison

> Comparison between the **Lavalink** reference server (Kotlin) and **Rustalink** (Rust).

---

## Op-Level Messages

These are the top-level WebSocket messages distinguished by the `op` field.

| # | Op Code | Event | Lavalink | Rustalink | Status |
|---|---------|-------|----------|-----------|--------|
| 1 | `ready` | **Ready** — Sent on WS connect (includes `resumed`, `sessionId`) | ✅ `SocketServer.kt` | ✅ `websocket_server.rs` | ✅ Fully Implemented |
| 2 | `playerUpdate` | **PlayerUpdate** — Periodic player state (position, connected, ping) | ✅ `SocketServer.kt` | ✅ `websocket_server.rs`, `playback.rs` | ✅ Fully Implemented |
| 3 | `stats` | **Stats** — Server stats (CPU, memory, players, uptime, frameStats) | ✅ `StatsCollector.kt` | ✅ `websocket_server.rs` + `monitoring/` | ✅ Fully Implemented |
| 4 | `event` | **Event** — Player events (see Emitted Events below) | ✅ `EventEmitter.kt` | ✅ `api/events.rs` | ⚠️ Partially (see below) |

---

## Emitted Events (`op: "event"`)

These are the events nested under `op: "event"`, differentiated by the `type` field.

### 1. `TrackStartEvent`

Emitted when a track begins playing.

| Field | Type | Description |
|-------|------|-------------|
| `op` | `"event"` | Op code |
| `type` | `"TrackStartEvent"` | Event type |
| `guildId` | `string` | Guild snowflake |
| `track` | `Track` | Full encoded track object |

| | Lavalink | Rustalink |
|---|----------|-----------|
| **Defined** | ✅ `messages.kt` L134 | ✅ `api/events.rs` L27-29 |
| **Emitted** | ✅ `player/EventEmitter.kt` L49 | ✅ `player/playback.rs` L125 |
| **Status** | | ✅ **Fully Implemented** |

---

### 2. `TrackEndEvent`

Emitted when a track stops playing.

| Field | Type | Description |
|-------|------|-------------|
| `op` | `"event"` | Op code |
| `type` | `"TrackEndEvent"` | Event type |
| `guildId` | `string` | Guild snowflake |
| `track` | `Track` | Full encoded track object |
| `reason` | `AudioTrackEndReason` | Why the track ended |

**End Reasons:**

| Reason | `mayStartNext` | Lavalink | Rustalink | Emitted At |
|--------|---------------|----------|-----------|------------|
| `finished` | `true` | ✅ | ✅ | `playback.rs` L153 — playback state → `Stopped` |
| `loadFailed` | `true` | ✅ | ⚠️ **Defined but never emitted** | Only defined in `events.rs` L68. When a track fails to resolve, `playback.rs` L94 just logs an error and returns silently — no `TrackEndEvent` with `loadFailed` is sent. |
| `stopped` | `false` | ✅ | ✅ | `routes/player/update.rs` L233 — REST PATCH with `track: null` |
| `replaced` | `false` | ✅ | ✅ | `playback.rs` L32 — when a new track replaces a playing one |
| `cleanup` | `false` | ✅ | ✅ | `routes/player/destroy.rs` L28 — player destruction |

| **Status** | | ⚠️ **Mostly Implemented** — `loadFailed` reason is never emitted |

---

### 3. `TrackExceptionEvent`

Emitted when a track throws an exception during playback.

| Field | Type | Description |
|-------|------|-------------|
| `op` | `"event"` | Op code |
| `type` | `"TrackExceptionEvent"` | Event type |
| `guildId` | `string` | Guild snowflake |
| `track` | `Track` | Full encoded track object |
| `exception.message` | `string?` | Error message |
| `exception.severity` | `string` | `common`, `suspicious`, or `fault` |
| `exception.cause` | `string` | Root cause class/message |

| | Lavalink | Rustalink |
|---|----------|-----------|
| **Defined** | ✅ `messages.kt` L202 | ✅ `api/events.rs` L37-44 (with `#[allow(dead_code)]`) |
| **Emitted** | ✅ `player/EventEmitter.kt` L78 | ❌ **Never emitted anywhere in the codebase** |
| **Status** | | ❌ **NOT Implemented** — struct exists but is dead code |

> [!CAUTION]
> Lavalink emits this when Lavaplayer's `onTrackException` fires. Rustalink has no equivalent error callback during playback — track decode/stream errors are logged but never surfaced to the client.

---

### 4. `TrackStuckEvent`

Emitted when a track gets stuck (no audio frames provided within threshold).

| Field | Type | Description |
|-------|------|-------------|
| `op` | `"event"` | Op code |
| `type` | `"TrackStuckEvent"` | Event type |
| `guildId` | `string` | Guild snowflake |
| `track` | `Track` | Full encoded track object |
| `thresholdMs` | `number` | Stuck threshold in milliseconds |

| | Lavalink | Rustalink |
|---|----------|-----------|
| **Defined** | ✅ `messages.kt` L215 | ✅ `api/events.rs` L45-52 (with `#[allow(dead_code)]`) |
| **Emitted** | ✅ `player/EventEmitter.kt` L91 | ❌ **Never emitted anywhere in the codebase** |
| **Status** | | ❌ **NOT Implemented** — struct exists but is dead code |

> [!CAUTION]
> Lavalink detects stuck tracks via Lavaplayer's `onTrackStuck` callback (default threshold: 10 seconds of no audio). Rustalink has no stuck detection mechanism — if the decoder stalls, no event is sent to the client.

---

### 5. `WebSocketClosedEvent`

Emitted when Discord's voice WebSocket connection is closed.

| Field | Type | Description |
|-------|------|-------------|
| `op` | `"event"` | Op code |
| `type` | `"WebSocketClosedEvent"` | Event type |
| `guildId` | `string` | Guild snowflake |
| `code` | `number` | WS close code (e.g., `4014`) |
| `reason` | `string` | Close reason |
| `byRemote` | `boolean` | Whether closed by Discord |

| | Lavalink | Rustalink |
|---|----------|-----------|
| **Defined** | ✅ `messages.kt` L230 | ✅ `api/events.rs` L53-61 (with `#[allow(dead_code)]`) |
| **Emitted** | ✅ `SocketContext.kt` L220 — via Koe's `gatewayClosed` callback | ❌ **Never emitted anywhere in the codebase** |
| **Status** | | ❌ **NOT Implemented** — struct exists but is dead code |

> [!CAUTION]
> Lavalink emits this via Koe's `KoeEventAdapter.gatewayClosed()` when Discord's voice gateway drops. Rustalink's `gateway/session.rs` handles voice WS closes internally (reconnect/shutdown logic at L651) but **never** forwards the close event to the Lavalink client.

---

## Summary

| Event | Lavalink | Rustalink | Gap |
|-------|----------|-----------|-----|
| `ready` | ✅ | ✅ | — |
| `playerUpdate` | ✅ | ✅ | — |
| `stats` | ✅ | ✅ | — |
| `TrackStartEvent` | ✅ | ✅ | — |
| `TrackEndEvent` (`finished`) | ✅ | ✅ | — |
| `TrackEndEvent` (`loadFailed`) | ✅ | ⚠️ Defined | Never emitted on load failure |
| `TrackEndEvent` (`stopped`) | ✅ | ✅ | — |
| `TrackEndEvent` (`replaced`) | ✅ | ✅ | — |
| `TrackEndEvent` (`cleanup`) | ✅ | ✅ | — |
| `TrackExceptionEvent` | ✅ | ❌ Dead code | No playback error callback exists |
| `TrackStuckEvent` | ✅ | ❌ Dead code | No stuck detection mechanism |
| `WebSocketClosedEvent` | ✅ | ❌ Dead code | Voice gateway close not forwarded |

### Missing Implementations (3 events)

1. **`TrackExceptionEvent`** — Need to catch decode/stream errors in `playback.rs` and emit the event with severity + cause
2. **`TrackStuckEvent`** — Need a stuck detection timer (e.g., 10s with no audio frames) in the playback loop
3. **`WebSocketClosedEvent`** — Need to emit this in `gateway/session.rs` when Discord's voice WS closes (requires passing the Lavalink session sender into the voice gateway)

### Partially Missing

4. **`TrackEndEvent` with `loadFailed`** — The track resolution failure path in `playback.rs` L94 silently returns. It should emit a `TrackEndEvent` with `reason: "loadFailed"` before returning.
