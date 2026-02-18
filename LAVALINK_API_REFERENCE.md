# Lavalink v4 — Complete API Reference for Rust Client Implementation

> **Target Audience:** Developers building a Lavalink client library in **Rust**.
> **Lavalink API Version:** v4 (routes prefixed with `/v4`, except `/version`).
> **Source of Truth:** [lavalink.dev/api](https://lavalink.dev/api/index.html)

---

## Table of Contents

1. [Connection & Authentication](#1-connection--authentication)
2. [Common Data Types](#2-common-data-types)
3. [WebSocket OP Types](#3-websocket-op-types)
4. [WebSocket Events](#4-websocket-events)
5. [REST API — Track Endpoints](#5-rest-api--track-endpoints)
6. [REST API — Player Endpoints](#6-rest-api--player-endpoints)
7. [REST API — Session Endpoints](#7-rest-api--session-endpoints)
8. [REST API — Info / Version / Stats](#8-rest-api--info--version--stats)
9. [REST API — RoutePlanner Endpoints](#9-rest-api--routeplanner-endpoints)
10. [REST API — Error Responses](#10-rest-api--error-responses)
11. [Filters (Audio Effects)](#11-filters-audio-effects)
12. [Session Resuming](#12-session-resuming)
13. [Player Lifecycle & Edge Cases](#13-player-lifecycle--edge-cases)
14. [Rust-Specific Implementation Notes](#14-rust-specific-implementation-notes)
15. [Implementation Checklist](#15-implementation-checklist)

---

## 1. Connection & Authentication

### WebSocket Endpoint

```
ws://<host>:<port>/v4/websocket
```

### Required Headers

| Header          | Type   | Required | Description                                                    |
|-----------------|--------|----------|----------------------------------------------------------------|
| `Authorization` | string | ✅       | The configured Lavalink password                               |
| `User-Id`       | string | ✅       | The Discord bot's user (snowflake) ID                          |
| `Client-Name`   | string | ✅       | Identifier in `NAME/VERSION` format (e.g. `rustalink/1.0.0`)       |
| `Session-Id`    | string | ❌       | Previous session ID — send this to **resume** a session        |

### REST Authentication

All REST routes require the same `Authorization` header:

```
Authorization: youshallnotpass
```

### Response Header (Resume)

| Header           | Value          | Description                    |
|------------------|----------------|--------------------------------|
| `Session-Resumed`| `true`/`false` | Whether the session was resumed |

---

## 2. Common Data Types

### Track

| Field        | Type        | Description                                                       |
|--------------|-------------|-------------------------------------------------------------------|
| `encoded`    | `String`    | Base64 encoded track data (opaque blob, used to play the track)   |
| `info`       | `TrackInfo` | Decoded metadata about the track                                  |
| `pluginInfo` | `Object`    | Additional info from plugins (`serde_json::Value` in Rust)        |
| `userData`   | `Object`    | User-supplied data via Update Player (`serde_json::Value`)        |

### TrackInfo

| Field        | Type      | Rust Type          | Description                              |
|--------------|-----------|--------------------|------------------------------------------|
| `identifier` | string    | `String`           | Platform-specific track ID               |
| `isSeekable` | bool      | `bool`             | Whether seeking is supported             |
| `author`     | string    | `String`           | Track author / artist                    |
| `length`     | int       | `u64`              | Duration in milliseconds                 |
| `isStream`   | bool      | `bool`             | Whether this is a livestream             |
| `position`   | int       | `u64`              | Current position in ms                   |
| `title`      | string    | `String`           | Track title                              |
| `uri`        | ?string   | `Option<String>`   | Track URL (may be null)                  |
| `artworkUrl` | ?string   | `Option<String>`   | Artwork/thumbnail URL                    |
| `isrc`       | ?string   | `Option<String>`   | International Standard Recording Code    |
| `sourceName` | string    | `String`           | Source manager name (e.g. `"youtube"`)   |

### PlaylistInfo

| Field           | Type   | Rust Type | Description                                         |
|-----------------|--------|-----------|-----------------------------------------------------|
| `name`          | string | `String`  | Playlist name                                       |
| `selectedTrack` | int    | `i32`     | Index of selected track (`-1` if none)              |

### PlayerState

| Field       | Type | Rust Type | Description                                     |
|-------------|------|-----------|-------------------------------------------------|
| `time`      | int  | `u64`     | Unix timestamp in ms of when state was sent     |
| `position`  | int  | `u64`     | Track position in ms                            |
| `connected` | bool | `bool`    | Whether the player is connected to Discord voice|
| `ping`      | int  | `i64`     | Ping to Discord voice server in ms (`-1` if N/A)|

### VoiceState

| Field       | Type   | Rust Type | Description                                |
|-------------|--------|-----------|--------------------------------------------|
| `token`     | string | `String`  | Discord voice token                        |
| `endpoint`  | string | `String`  | Discord voice endpoint                     |
| `sessionId` | string | `String`  | Discord voice session ID                   |

> **Note:** These 3 values come from Discord's `VOICE_SERVER_UPDATE` (`token` + `endpoint`) and `VOICE_STATE_UPDATE` (`sessionId`) gateway events. You must intercept these from your Discord gateway connection and forward them to Lavalink.

### Exception

| Field             | Type     | Rust Type        | Description                    |
|-------------------|----------|------------------|--------------------------------|
| `message`         | ?string  | `Option<String>` | Error message                  |
| `severity`        | string   | `Severity`       | See Severity enum below        |
| `cause`           | ?string  | `Option<String>` | Error cause                    |
| `causeStackTrace` | ?string  | `Option<String>` | Java stack trace (for debugging) |

### Severity (enum)

| Value        | Description                                                              |
|--------------|--------------------------------------------------------------------------|
| `common`     | Common error, track failed to load (e.g. YouTube age-restricted)         |
| `suspicious` | Suspicious error, may indicate a Lavalink issue                          |
| `fault`      | Fatal error, something is very wrong                                     |

---

## 3. WebSocket OP Types

All messages from the Lavalink WebSocket follow this envelope:

```json
{ "op": "<op_type>", ... }
```

| OP Type        | Direction       | Description                                   |
|----------------|-----------------|-----------------------------------------------|
| `ready`        | Server → Client | Sent on successful connection/resume          |
| `playerUpdate` | Server → Client | Periodic player state update                  |
| `stats`        | Server → Client | Server statistics sent every ~60 seconds      |
| `event`        | Server → Client | Player event (track start/end/error/etc.)     |

### Ready OP

Dispatched upon successful WebSocket connection and authorization.

```json
{
  "op": "ready",
  "resumed": false,
  "sessionId": "la4kj8hf90ah"
}
```

| Field       | Type   | Rust Type | Description                               |
|-------------|--------|-----------|-------------------------------------------|
| `op`        | string | —         | Always `"ready"`                          |
| `resumed`   | bool   | `bool`    | `true` if this is a resumed session       |
| `sessionId` | string | `String`  | Session ID — **store this** for REST calls and future resume |

### Player Update OP

Dispatched periodically (configurable in `application.yml`) with the current state of a player.

```json
{
  "op": "playerUpdate",
  "guildId": "725429036300066826",
  "state": {
    "time": 1500467109,
    "position": 60000,
    "connected": true,
    "ping": 50
  }
}
```

| Field     | Type        | Rust Type     | Description                  |
|-----------|-------------|---------------|------------------------------|
| `op`      | string      | —             | Always `"playerUpdate"`      |
| `guildId` | string      | `String`      | The guild this player is in  |
| `state`   | PlayerState | `PlayerState` | Current player state         |

### Stats OP

Server statistics dispatched every ~60 seconds.

```json
{
  "op": "stats",
  "players": 1,
  "playingPlayers": 1,
  "uptime": 123456789,
  "memory": {
    "free": 123456789,
    "used": 123456789,
    "allocated": 123456789,
    "reservable": 123456789
  },
  "cpu": {
    "cores": 4,
    "systemLoad": 0.5,
    "lavalinkLoad": 0.5
  },
  "frameStats": {
    "sent": 6000,
    "nulled": 10,
    "deficit": -3010
  }
}
```

| Field            | Type       | Rust Type          | Description                         |
|------------------|------------|--------------------|-------------------------------------|
| `op`             | string     | —                  | Always `"stats"`                    |
| `players`        | int        | `u32`              | Total connected players             |
| `playingPlayers` | int        | `u32`              | Players currently playing a track   |
| `uptime`         | int        | `u64`              | Server uptime in ms                 |
| `memory`         | Memory     | `Memory`           | JVM memory stats                    |
| `cpu`            | Cpu        | `Cpu`              | CPU stats                           |
| `frameStats`     | ?FrameStats| `Option<FrameStats>`| Audio frame stats (nullable)       |

#### Memory

| Field        | Type | Rust Type | Description              |
|--------------|------|-----------|--------------------------|
| `free`       | int  | `u64`     | Free JVM memory (bytes)  |
| `used`       | int  | `u64`     | Used JVM memory (bytes)  |
| `allocated`  | int  | `u64`     | Allocated memory (bytes) |
| `reservable` | int  | `u64`     | Reservable memory (bytes)|

#### Cpu

| Field          | Type  | Rust Type | Description                   |
|----------------|-------|-----------|-------------------------------|
| `cores`        | int   | `u32`     | Number of CPU cores           |
| `systemLoad`   | float | `f64`     | System CPU load (0.0–1.0)     |
| `lavalinkLoad` | float | `f64`     | Lavalink process CPU load     |

#### FrameStats

| Field     | Type | Rust Type | Description                                                                     |
|-----------|------|-----------|---------------------------------------------------------------------------------|
| `sent`    | int  | `i32`     | Frames sent in the interval                                                     |
| `nulled`  | int  | `i32`     | Frames nulled (silence sent because no data available)                          |
| `deficit` | int  | `i32`     | Frame deficit. Expected is 3000/min (1 per 20ms). Positive = not enough sent   |

---

## 4. WebSocket Events

All events share this envelope:

```json
{
  "op": "event",
  "type": "<EventType>",
  "guildId": "<snowflake>",
  ...
}
```

### 4.1 TrackStartEvent

**Triggered when:** A track begins playing on a player.

```json
{
  "op": "event",
  "type": "TrackStartEvent",
  "guildId": "725429036300066826",
  "track": {
    "encoded": "QAAAjQIAJVJpY2sg...",
    "info": {
      "identifier": "dQw4w9WgXcQ",
      "isSeekable": true,
      "author": "RickAstleyVEVO",
      "length": 212000,
      "isStream": false,
      "position": 0,
      "title": "Rick Astley - Never Gonna Give You Up",
      "uri": "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
      "artworkUrl": "https://i.ytimg.com/vi/dQw4w9WgXcQ/maxresdefault.jpg",
      "isrc": null,
      "sourceName": "youtube"
    },
    "pluginInfo": {},
    "userData": {}
  }
}
```

| Field     | Type   | Description                     |
|-----------|--------|---------------------------------|
| `type`    | string | Always `"TrackStartEvent"`      |
| `guildId` | string | Guild snowflake ID              |
| `track`   | Track  | The track that started playing  |

---

### 4.2 TrackEndEvent

**Triggered when:** A track finishes, is stopped, fails to load, gets replaced, or player is cleaned up.

```json
{
  "op": "event",
  "type": "TrackEndEvent",
  "guildId": "725429036300066826",
  "track": {
    "encoded": "QAAAjQIAJVJpY2sg...",
    "info": { ... },
    "pluginInfo": {},
    "userData": {}
  },
  "reason": "finished"
}
```

| Field     | Type            | Description                    |
|-----------|-----------------|--------------------------------|
| `type`    | string          | Always `"TrackEndEvent"`       |
| `guildId` | string          | Guild snowflake ID             |
| `track`   | Track           | The track that ended           |
| `reason`  | TrackEndReason  | Why the track ended            |

#### TrackEndReason (enum)

| Value        | May Start Next | Description                                              |
|--------------|:--------------:|----------------------------------------------------------|
| `finished`   | ✅             | Track completed normally — **load next track**           |
| `loadFailed` | ✅             | Track failed to load/produce audio — **try next track**  |
| `stopped`    | ❌             | Track was explicitly stopped (e.g. via API)              |
| `replaced`   | ❌             | Track was replaced by another track                      |
| `cleanup`    | ❌             | Player was destroyed or node disconnected                |

> **Critical Rust note:** Only queue the next track when `reason` is `finished` or `loadFailed`. For `stopped`, `replaced`, and `cleanup`, the end was intentional — starting a new track would conflict with the user's intent.

---

### 4.3 TrackExceptionEvent

**Triggered when:** A track throws an exception during playback.

```json
{
  "op": "event",
  "type": "TrackExceptionEvent",
  "guildId": "725429036300066826",
  "track": {
    "encoded": "QAAAjQIAJVJpY2sg...",
    "info": { ... },
    "pluginInfo": {}
  },
  "exception": {
    "message": "This video is not available",
    "severity": "common",
    "cause": "com.sedmelluq.discord.lavaplayer.tools.FriendlyException",
    "causeStackTrace": "..."
  }
}
```

| Field       | Type      | Description                       |
|-------------|-----------|-----------------------------------|
| `type`      | string    | Always `"TrackExceptionEvent"`    |
| `guildId`   | string    | Guild snowflake ID                |
| `track`     | Track     | The track that errored            |
| `exception` | Exception | Exception details with severity   |

> **Note:** A `TrackEndEvent` with `reason: "loadFailed"` will **also** fire after this event.

---

### 4.4 TrackStuckEvent

**Triggered when:** A track gets stuck (audio hasn't been provided for a threshold period).

```json
{
  "op": "event",
  "type": "TrackStuckEvent",
  "guildId": "725429036300066826",
  "track": {
    "encoded": "QAAAjQIAJVJpY2sg...",
    "info": { ... },
    "pluginInfo": {}
  },
  "thresholdMs": 10000
}
```

| Field         | Type   | Rust Type | Description                              |
|---------------|--------|-----------|------------------------------------------|
| `type`        | string | —         | Always `"TrackStuckEvent"`               |
| `guildId`     | string | `String`  | Guild snowflake ID                       |
| `track`       | Track  | `Track`   | The stuck track                          |
| `thresholdMs` | int    | `u64`     | Threshold in ms after which it's "stuck" |

> **Recommended handling:** Stop the current track and attempt the next in queue, or notify the user.

---

### 4.5 WebSocketClosedEvent

**Triggered when:** The audio WebSocket connection **to Discord** (not to Lavalink) is closed. This does **not** mean the Lavalink WS connection died.

```json
{
  "op": "event",
  "type": "WebSocketClosedEvent",
  "guildId": "725429036300066826",
  "code": 4006,
  "reason": "Your session is no longer valid.",
  "byRemote": true
}
```

| Field      | Type   | Rust Type | Description                                         |
|------------|--------|-----------|-----------------------------------------------------|
| `type`     | string | —         | Always `"WebSocketClosedEvent"`                     |
| `guildId`  | string | `String`  | Guild snowflake ID                                  |
| `code`     | int    | `u16`     | Discord voice close code (4xxx = usually bad)       |
| `reason`   | string | `String`  | Human-readable close reason                         |
| `byRemote` | bool   | `bool`    | `true` if Discord closed the connection             |

> **See:** [Discord Voice Close Event Codes](https://discord.com/developers/docs/topics/opcodes-and-status-codes#voice-voice-close-event-codes)

---

## 5. REST API — Track Endpoints

### 5.1 Load Tracks

Resolve audio tracks for playback.

```
GET /v4/loadtracks?identifier={identifier}
```

**Search prefixes:**
| Prefix       | Source         |
|--------------|---------------|
| `ytsearch:`  | YouTube        |
| `ytmsearch:` | YouTube Music  |
| `scsearch:`  | Soundcloud     |

**Response:** `TrackLoadingResult`

| Field      | Type             | Description          |
|------------|------------------|----------------------|
| `loadType` | `LoadResultType` | Type of result       |
| `data`     | varies           | Result data          |

#### LoadResultType (enum)

| Value      | `data` Type         | Description                        |
|------------|---------------------|------------------------------------|
| `track`    | `Track`             | A single track was loaded          |
| `playlist` | `PlaylistResult`    | A playlist was loaded              |
| `search`   | `Vec<Track>`        | Search results                     |
| `empty`    | `{}`                | No matches found                   |
| `error`    | `Exception`         | Loading failed                     |

##### Track Result

```json
{
  "loadType": "track",
  "data": {
    "encoded": "QAAAjQIAJVJpY2sg...",
    "info": { ... },
    "pluginInfo": {},
    "userData": {}
  }
}
```

##### Playlist Result

```json
{
  "loadType": "playlist",
  "data": {
    "info": {
      "name": "My Playlist",
      "selectedTrack": 0
    },
    "pluginInfo": {},
    "tracks": [ { "encoded": "...", "info": { ... } }, ... ]
  }
}
```

##### Search Result

```json
{
  "loadType": "search",
  "data": [
    { "encoded": "...", "info": { ... }, "pluginInfo": {}, "userData": {} },
    ...
  ]
}
```

##### Empty Result

```json
{ "loadType": "empty", "data": {} }
```

##### Error Result

```json
{
  "loadType": "error",
  "data": {
    "message": "Something went wrong",
    "severity": "fault",
    "cause": "...",
    "causeStackTrace": "..."
  }
}
```

---

### 5.2 Decode Track

```
GET /v4/decodetrack?encodedTrack={BASE64}
```

**Response:** `Track` object (full info, pluginInfo, userData).

---

### 5.3 Decode Tracks (Batch)

```
POST /v4/decodetracks
Content-Type: application/json
```

**Request Body:** `Vec<String>` — array of base64 encoded track strings.

**Response:** `Vec<Track>` — array of decoded Track objects.

---

## 6. REST API — Player Endpoints

### 6.1 Get Players

```
GET /v4/sessions/{sessionId}/players
```

**Response:** `Vec<Player>` — all players in this session.

### 6.2 Get Player

```
GET /v4/sessions/{sessionId}/players/{guildId}
```

**Response:** `Player` object.

### Player Object

| Field     | Type        | Rust Type             | Description                          |
|-----------|-------------|-----------------------|--------------------------------------|
| `guildId` | string      | `String`              | Guild snowflake ID                   |
| `track`   | ?Track      | `Option<Track>`       | Currently playing track (null if none)|
| `volume`  | int         | `u16`                 | Volume 0–1000 (percentage)           |
| `paused`  | bool        | `bool`                | Whether player is paused             |
| `state`   | PlayerState | `PlayerState`         | Current player state                 |
| `voice`   | VoiceState  | `VoiceState`          | Discord voice connection info        |
| `filters` | Filters     | `Filters`             | Applied audio filters                |

---

### 6.3 Update Player

Creates or updates the player for a guild.

```
PATCH /v4/sessions/{sessionId}/players/{guildId}?noReplace=true
```

> **Important:** `sessionId` must be the value received from the WebSocket Ready OP.

**Query Params:**

| Param        | Type | Default | Description                                              |
|--------------|------|---------|----------------------------------------------------------|
| `noReplace`  | bool | `false` | If `true`, don't replace the currently playing track     |

**Request Body:**

| Field                | Type             | Required | Description                                                       |
|----------------------|------------------|----------|-------------------------------------------------------------------|
| `track`              | UpdatePlayerTrack| ❌       | New track to load + optional userData                             |
| ~~`encodedTrack`~~   | ?string          | ❌       | **(deprecated)** Base64 encoded track. `null` = stop              |
| ~~`identifier`~~     | string           | ❌       | **(deprecated)** Track identifier to resolve                      |
| `position`           | int              | ❌       | Seek to position in ms                                            |
| `endTime`            | ?int             | ❌       | Stop at this position in ms. `null` removes previous endTime      |
| `volume`             | int              | ❌       | Volume 0–1000                                                     |
| `paused`             | bool             | ❌       | Pause/unpause                                                     |
| `filters`            | Filters          | ❌       | Overwrite all filters                                             |
| `voice`              | VoiceState       | ❌       | Discord voice connection data                                     |

#### UpdatePlayerTrack

| Field          | Type    | Description                                                  |
|----------------|---------|--------------------------------------------------------------|
| `encoded`*     | ?string | Base64 encoded track. `null` = stop current track            |
| `identifier`*  | string  | Track identifier to resolve as single track                  |
| `userData`     | object  | Custom data attached to the track                            |

> *`encoded` and `identifier` are **mutually exclusive**. When `identifier` is used, Lavalink resolves it as a single track; playlists/search results return HTTP 400.

**Response:** `Player` object (the updated player).

---

### 6.4 Destroy Player

```
DELETE /v4/sessions/{sessionId}/players/{guildId}
```

**Response:** `204 No Content`

> This fully destroys the player, closes the voice connection, and cleans up all resources. You will receive a `TrackEndEvent` with `reason: "cleanup"` if a track was playing.

---

## 7. REST API — Session Endpoints

### 7.1 Update Session

Configure session resuming.

```
PATCH /v4/sessions/{sessionId}
Content-Type: application/json
```

**Request Body:**

| Field      | Type | Description                                          |
|------------|------|------------------------------------------------------|
| `resuming` | bool | Enable/disable session resuming                      |
| `timeout`  | int  | Timeout in seconds before session expires (default: 60) |

```json
{
  "resuming": true,
  "timeout": 60
}
```

**Response:**

```json
{
  "resuming": true,
  "timeout": 60
}
```

---

## 8. REST API — Info / Version / Stats

### 8.1 Get Lavalink Info

```
GET /v4/info
```

**Response:**

| Field            | Type           | Rust Type       | Description                           |
|------------------|----------------|-----------------|---------------------------------------|
| `version`        | Version        | `Version`       | Lavalink server version               |
| `buildTime`      | int            | `u64`           | Unix timestamp (ms) of JAR build      |
| `git`            | Git            | `Git`           | Git build info                        |
| `jvm`            | string         | `String`        | JVM version                           |
| `lavaplayer`     | string         | `String`        | Lavaplayer version                    |
| `sourceManagers` | string[]       | `Vec<String>`   | Enabled source managers               |
| `filters`        | string[]       | `Vec<String>`   | Enabled filter types                  |
| `plugins`        | Plugin[]       | `Vec<Plugin>`   | Loaded plugins                        |

#### Version

| Field        | Type    | Rust Type        | Description             |
|--------------|---------|------------------|-------------------------|
| `semver`     | string  | `String`         | Full semver string      |
| `major`      | int     | `u32`            | Major version           |
| `minor`      | int     | `u32`            | Minor version           |
| `patch`      | int     | `u32`            | Patch version           |
| `preRelease` | ?string | `Option<String>` | Pre-release identifier  |
| `build`      | ?string | `Option<String>` | Build metadata          |

#### Git

| Field        | Type   | Rust Type | Description                       |
|--------------|--------|-----------|-----------------------------------|
| `branch`     | string | `String`  | Build branch                      |
| `commit`     | string | `String`  | Commit hash                       |
| `commitTime` | int    | `u64`     | Commit timestamp (ms)             |

#### Plugin

| Field     | Type   | Rust Type | Description       |
|-----------|--------|-----------|-------------------|
| `name`    | string | `String`  | Plugin name       |
| `version` | string | `String`  | Plugin version    |

**Example:**

```json
{
  "version": {
    "semver": "4.0.0",
    "major": 4,
    "minor": 0,
    "patch": 0,
    "preRelease": null,
    "build": null
  },
  "buildTime": 1664223916812,
  "git": {
    "branch": "master",
    "commit": "85c5ab5",
    "commitTime": 1664223916812
  },
  "jvm": "18.0.2.1",
  "lavaplayer": "1.3.98.4-original",
  "sourceManagers": ["youtube", "soundcloud"],
  "filters": ["equalizer", "karaoke", "timescale", "channelMix"],
  "plugins": [
    { "name": "some-plugin", "version": "1.0.0" }
  ]
}
```

---

### 8.2 Get Lavalink Version

```
GET /version
```

> **Note:** This endpoint is NOT prefixed with `/v4`.

**Response:** Plain text version string (e.g. `4.0.0`).

---

### 8.3 Get Lavalink Stats

```
GET /v4/stats
```

**Response:** Same as the [Stats OP](#stats-op) object, but `frameStats` is **always `null`** for this endpoint.

---

## 9. REST API — RoutePlanner Endpoints

These endpoints manage IP rotation for avoiding rate limits.

### Route Planner Types (enum)

| Value                          | Description                                                     |
|--------------------------------|-----------------------------------------------------------------|
| `RotatingIpRoutePlanner`       | Switches IP on ban. For IPv4 or small IPv6 blocks.              |
| `NanoIpRoutePlanner`           | Switches IP on clock update. Needs ≥1 /64 IPv6 block.          |
| `RotatingNanoIpRoutePlanner`   | Clock-based switching + /64 block rotation on ban. Needs ≥2.    |
| `BalancingIpRoutePlanner`      | Random IP selection per request. For large blocks.              |

### 9.1 Get RoutePlanner Status

```
GET /v4/routeplanner/status
```

**Responses:**
- `204 No Content` — RoutePlanner not enabled
- `200` with body:

| Field     | Type              | Description                          |
|-----------|-------------------|--------------------------------------|
| `class`   | ?RoutePlannerType | RoutePlanner implementation in use   |
| `details` | ?Details          | Status details                       |

#### Details

| Field                 | Type           | Applies To                          | Description                    |
|-----------------------|----------------|-------------------------------------|--------------------------------|
| `ipBlock`             | IpBlock        | all                                 | IP block in use                |
| `failingAddresses`    | FailingAddress[]| all                                | Currently failing addresses    |
| `rotateIndex`         | string         | `RotatingIpRoutePlanner`            | Number of rotations            |
| `ipIndex`             | string         | `RotatingIpRoutePlanner`            | Current offset in block        |
| `currentAddress`      | string         | `RotatingIpRoutePlanner`            | Current address                |
| `currentAddressIndex` | string         | `NanoIp`, `RotatingNanoIp`          | Current offset in IP block     |
| `blockIndex`          | string         | `RotatingNanoIpRoutePlanner`        | Current /64 block index        |

#### IpBlock

| Field  | Type   | Description                                |
|--------|--------|--------------------------------------------|
| `type` | string | `"Inet4Address"` or `"Inet6Address"`       |
| `size` | string | Size as string (can be very large for IPv6)|

#### FailingAddress

| Field              | Type   | Description                |
|--------------------|--------|----------------------------|
| `failingAddress`   | string | The failing IP address     |
| `failingTimestamp`  | int    | Failure timestamp (ms)     |
| `failingTime`      | string | Human-readable timestamp   |

---

### 9.2 Unmark Failed Address

```
POST /v4/routeplanner/free/address
Content-Type: application/json
```

**Request:**

```json
{ "address": "1.0.0.1" }
```

**Response:** `204 No Content`

---

### 9.3 Unmark All Failed Addresses

```
POST /v4/routeplanner/free/all
```

**Response:** `204 No Content`

---

## 10. REST API — Error Responses

All REST errors return this JSON structure:

| Field       | Type    | Description                                                          |
|-------------|---------|----------------------------------------------------------------------|
| `timestamp` | int     | Unix timestamp in ms                                                 |
| `status`    | int     | HTTP status code                                                     |
| `error`     | string  | HTTP status message (e.g. `"Not Found"`)                             |
| `trace`     | ?string | Full stack trace (only if `?trace=true` query param sent)            |
| `message`   | string  | Error message                                                        |
| `path`      | string  | Request path                                                         |

```json
{
  "timestamp": 1667857581613,
  "status": 404,
  "error": "Not Found",
  "trace": "...",
  "message": "Session not found",
  "path": "/v4/sessions/xtaug914v9k5032f/players/817327181659111454"
}
```

---

## 11. Filters (Audio Effects)

All filters are optional. Set via the `filters` field in [Update Player](#63-update-player).

### Top-Level Filters Object

| Field            | Type            | Description                                             |
|------------------|-----------------|---------------------------------------------------------|
| `volume`         | ?float          | Volume 0.0–5.0 (1.0 = 100%). Values >1.0 may clip.    |
| `equalizer`      | ?Equalizer[]    | Array of up to 15 EQ bands                             |
| `karaoke`        | ?Karaoke        | Vocal elimination filter                                |
| `timescale`      | ?Timescale      | Speed/pitch/rate adjustment                             |
| `tremolo`        | ?Tremolo        | Volume oscillation effect                               |
| `vibrato`        | ?Vibrato        | Pitch oscillation effect                                |
| `rotation`       | ?Rotation       | Audio panning / spatial rotation                        |
| `distortion`     | ?Distortion     | Audio distortion                                        |
| `channelMix`     | ?ChannelMix     | L/R channel mixing                                      |
| `lowPass`        | ?LowPass        | Low-pass frequency filter                               |
| `pluginFilters`  | ?Map<String, V> | Plugin-specific filters                                 |

### Equalizer Band

| Field  | Type  | Range          | Description                    |
|--------|-------|----------------|--------------------------------|
| `band` | int   | 0–14           | Band index (see frequencies)   |
| `gain` | float | -0.25 to 1.0   | Gain multiplier (0 = default)  |

**Band Frequencies:** 25, 40, 63, 100, 160, 250, 400, 630, 1000, 1600, 2500, 4000, 6300, 10000, 16000 Hz

### Karaoke

| Field         | Type  | Description                              |
|---------------|-------|------------------------------------------|
| `level`       | float | Effect level (0.0–1.0)                   |
| `monoLevel`   | float | Mono effect level (0.0–1.0)              |
| `filterBand`  | float | Target frequency band (Hz)               |
| `filterWidth` | float | Width of the filter                      |

### Timescale

| Field   | Type  | Default | Constraint | Description      |
|---------|-------|---------|------------|------------------|
| `speed` | float | 1.0     | ≥ 0.0      | Playback speed   |
| `pitch` | float | 1.0     | ≥ 0.0      | Audio pitch      |
| `rate`  | float | 1.0     | ≥ 0.0      | Audio rate       |

### Tremolo

| Field       | Type  | Constraint      | Description         |
|-------------|-------|-----------------|---------------------|
| `frequency` | float | > 0.0           | Oscillation freq    |
| `depth`     | float | 0.0 < x ≤ 1.0  | Effect depth        |

### Vibrato

| Field       | Type  | Constraint       | Description          |
|-------------|-------|------------------|----------------------|
| `frequency` | float | 0.0 < x ≤ 14.0  | Oscillation freq     |
| `depth`     | float | 0.0 < x ≤ 1.0   | Effect depth         |

### Rotation

| Field        | Type  | Description                                    |
|--------------|-------|------------------------------------------------|
| `rotationHz` | float | Rotation speed in Hz (0.2 is a good starting value) |

### Distortion

| Field       | Type  | Description    |
|-------------|-------|----------------|
| `sinOffset` | float | Sin offset     |
| `sinScale`  | float | Sin scale      |
| `cosOffset` | float | Cos offset     |
| `cosScale`  | float | Cos scale      |
| `tanOffset` | float | Tan offset     |
| `tanScale`  | float | Tan scale      |
| `offset`    | float | Global offset  |
| `scale`     | float | Global scale   |

### ChannelMix

| Field          | Type  | Default | Range     | Description               |
|----------------|-------|---------|-----------|---------------------------|
| `leftToLeft`   | float | 1.0     | 0.0–1.0   | Left → Left factor        |
| `leftToRight`  | float | 0.0     | 0.0–1.0   | Left → Right factor       |
| `rightToLeft`  | float | 0.0     | 0.0–1.0   | Right → Left factor       |
| `rightToRight` | float | 1.0     | 0.0–1.0   | Right → Right factor      |

### LowPass

| Field       | Type  | Description                                          |
|-------------|-------|------------------------------------------------------|
| `smoothing` | float | Smoothing factor. Values ≤ 1.0 disable the filter.   |

### Full Filter Example

```json
{
  "volume": 1.0,
  "equalizer": [{ "band": 0, "gain": 0.2 }],
  "karaoke": { "level": 1.0, "monoLevel": 1.0, "filterBand": 220.0, "filterWidth": 100.0 },
  "timescale": { "speed": 1.0, "pitch": 1.0, "rate": 1.0 },
  "tremolo": { "frequency": 2.0, "depth": 0.5 },
  "vibrato": { "frequency": 2.0, "depth": 0.5 },
  "rotation": { "rotationHz": 0.0 },
  "distortion": {
    "sinOffset": 0.0, "sinScale": 1.0, "cosOffset": 0.0, "cosScale": 1.0,
    "tanOffset": 0.0, "tanScale": 1.0, "offset": 0.0, "scale": 1.0
  },
  "channelMix": { "leftToLeft": 1.0, "leftToRight": 0.0, "rightToLeft": 0.0, "rightToRight": 1.0 },
  "lowPass": { "smoothing": 20.0 },
  "pluginFilters": {}
}
```

---

## 12. Session Resuming

### How It Works

1. Connect via WebSocket → receive `Ready OP` with `sessionId`.
2. Call `PATCH /v4/sessions/{sessionId}` with `{ "resuming": true, "timeout": 60 }`.
3. If your client disconnects:
   - **Resuming disabled:** All voice connections are closed immediately.
   - **Resuming enabled:** Music continues playing. Events are queued.
4. To resume: Reconnect WebSocket with `Session-Id: <old sessionId>` header.
5. Check `Session-Resumed` response header or `Ready OP`'s `resumed` field.
6. Queued events are replayed in order upon successful resume.

### Timeout Behavior

If the client doesn't reconnect within `timeout` seconds, the session is destroyed and all players are cleaned up.

### Special Notes

- When your **Discord gateway shard** WS dies, all Lavalink audio connections for that shard die too (even during resumes).
- If **Lavalink server** dies unexpectedly (SIGKILL), your client must clean up by sending Discord a voice disconnect:

```json
{
  "op": 4,
  "d": {
    "self_deaf": false,
    "guild_id": "GUILD_ID_HERE",
    "channel_id": null,
    "self_mute": false
  }
}
```

---

## 13. Player Lifecycle & Edge Cases

### Destroy vs. Stop vs. Pause

| Action                | How                                                          | Effect                                                                             |
|-----------------------|--------------------------------------------------------------|------------------------------------------------------------------------------------|
| **Pause**             | `PATCH .../players/{guildId}` with `{ "paused": true }`      | Audio stops sending. Track position preserved. Resume with `paused: false`.        |
| **Stop**              | `PATCH .../players/{guildId}` with `{ "track": { "encoded": null } }` | Current track stops. Player stays alive/connected. Emits `TrackEndEvent(stopped)`. |
| **Destroy**           | `DELETE .../players/{guildId}`                               | Player fully removed. Voice disconnected. Emits `TrackEndEvent(cleanup)`.          |

### Track Finish Flow

1. Track plays to completion
2. `TrackEndEvent` with `reason: "finished"` is emitted
3. Your client should load/play the next track from its queue
4. If no next track, player sits idle (still connected to voice)

### Track Error Flow

1. Track encounters an error during playback
2. `TrackExceptionEvent` is emitted with error details
3. `TrackEndEvent` with `reason: "loadFailed"` follows immediately
4. Your client should attempt the next track or notify the user

### Track Stuck Flow

1. Track stops providing audio frames for `thresholdMs` milliseconds
2. `TrackStuckEvent` is emitted
3. No automatic `TrackEndEvent` follows — your client should decide:
   - Stop/skip the current track
   - Retry the same track
   - Notify the user

### Bot Disconnect / Voice State

- When a bot is moved to a different voice channel, Discord sends new `VOICE_SERVER_UPDATE` / `VOICE_STATE_UPDATE` events. Forward the new `token`, `endpoint`, and `sessionId` to Lavalink via Update Player.
- When a bot is disconnected from voice (kicked or `channel_id: null`), you should destroy the player.
- Lavalink does **not** handle joining/leaving voice channels — your Discord library must do that via Gateway OP 4.

### Common Pitfalls

1. **Not intercepting voice events:** You MUST capture `VOICE_SERVER_UPDATE` and `VOICE_STATE_UPDATE` from Discord and send `token`, `endpoint`, `sessionId` to Lavalink.
2. **Trying to connect to voice via your Discord library:** Let Lavalink handle the voice connection. Only send Gateway OP 4 to join a channel.
3. **Playing audio without joining a channel first.**
4. **Starting a new track on `TrackEndEvent` with `stopped`/`replaced`/`cleanup` reason** — this conflicts with the user's intent.

---

## 14. Rust-Specific Implementation Notes

### Serde Deserialization Strategy

#### WebSocket Message (Tagged Union)

Use `#[serde(tag = "op")]` for the top-level OP dispatch:

```rust
#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum LavalinkMessage {
    Ready(ReadyOp),
    PlayerUpdate(PlayerUpdateOp),
    Stats(StatsOp),
    Event(EventOp),
}
```

#### Event Dispatch (Internally Tagged)

Use `#[serde(tag = "type")]` for event sub-dispatch:

```rust
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum EventOp {
    TrackStartEvent(TrackStartEvent),
    TrackEndEvent(TrackEndEvent),
    TrackExceptionEvent(TrackExceptionEvent),
    TrackStuckEvent(TrackStuckEvent),
    WebSocketClosedEvent(WebSocketClosedEvent),
}
```

#### TrackEndReason Enum

```rust
#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum TrackEndReason {
    Finished,
    LoadFailed,
    Stopped,
    Replaced,
    Cleanup,
}

impl TrackEndReason {
    pub fn may_start_next(&self) -> bool {
        matches!(self, Self::Finished | Self::LoadFailed)
    }
}
```

#### Severity Enum

```rust
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Severity {
    Common,
    Suspicious,
    Fault,
}
```

#### LoadResult (Externally Tagged via loadType)

```rust
#[derive(Debug, Deserialize)]
#[serde(tag = "loadType", content = "data", rename_all = "camelCase")]
pub enum LoadResult {
    Track(Track),
    Playlist(PlaylistData),
    Search(Vec<Track>),
    Empty(serde_json::Value),
    Error(Exception),
}
```

#### Nullable / Optional Fields

Lavalink uses two conventions:
- **`?` prefix on field name** (in docs) = optional (may be absent from JSON) → use `#[serde(default)]` + `Option<T>`
- **`?` prefix on type** (in docs) = nullable (present in JSON but may be `null`) → use `Option<T>`

```rust
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackInfo {
    pub identifier: String,
    pub is_seekable: bool,
    pub author: String,
    pub length: u64,
    pub is_stream: bool,
    pub position: u64,
    pub title: String,
    pub uri: Option<String>,        // nullable
    pub artwork_url: Option<String>, // nullable
    pub isrc: Option<String>,       // nullable
    pub source_name: String,
}
```

#### Plugin Info & User Data

These are arbitrary JSON objects — use `serde_json::Value`:

```rust
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub encoded: String,
    pub info: TrackInfo,
    #[serde(default)]
    pub plugin_info: serde_json::Value,
    #[serde(default)]
    pub user_data: serde_json::Value,
}
```

#### Filters — Skip Serialization of None

Use `#[serde(skip_serializing_if = "Option::is_none")]` on every filter field so only changed filters are sent:

```rust
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Filters {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub equalizer: Option<Vec<EqualizerBand>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub karaoke: Option<Karaoke>,
    // ... etc
}
```

#### Recommended Crates

| Crate                  | Purpose                                     |
|------------------------|---------------------------------------------|
| `serde` + `serde_json` | JSON serialization/deserialization           |
| `tokio-tungstenite`    | Async WebSocket client                      |
| `reqwest`              | HTTP client for REST API                    |
| `tokio`                | Async runtime                               |
| `tracing`              | Structured logging                          |
| `url`                  | URL building                                |

---

## 15. Implementation Checklist

### WebSocket Connection
- [ ] Connect to `/v4/websocket` with required headers
- [ ] Handle `Ready OP` → store `sessionId`
- [ ] Handle `PlayerUpdate OP` → update cached player state
- [ ] Handle `Stats OP` → update server stats
- [ ] Handle WebSocket reconnection with exponential backoff
- [ ] Implement session resume (send `Session-Id` header)

### WebSocket Events
- [ ] `TrackStartEvent` — notify queue/UI
- [ ] `TrackEndEvent` — handle all 5 reasons:
  - [ ] `finished` → play next
  - [ ] `loadFailed` → play next / retry
  - [ ] `stopped` → do nothing
  - [ ] `replaced` → do nothing
  - [ ] `cleanup` → clean up local state
- [ ] `TrackExceptionEvent` — log error, optionally notify user
- [ ] `TrackStuckEvent` — skip/retry stuck track
- [ ] `WebSocketClosedEvent` — log, handle 4xxx codes

### REST — Track API
- [ ] `GET /v4/loadtracks` — load/search tracks
- [ ] Handle all `LoadResultType` variants (`track`, `playlist`, `search`, `empty`, `error`)
- [ ] `GET /v4/decodetrack` — decode single track
- [ ] `POST /v4/decodetracks` — decode batch tracks

### REST — Player API
- [ ] `GET /v4/sessions/{sid}/players` — list players
- [ ] `GET /v4/sessions/{sid}/players/{gid}` — get player
- [ ] `PATCH /v4/sessions/{sid}/players/{gid}` — update player
  - [ ] Play track (via `track.encoded`)
  - [ ] Play by identifier (via `track.identifier`)
  - [ ] Stop track (via `track.encoded: null`)
  - [ ] Seek (via `position`)
  - [ ] Set `endTime`
  - [ ] Set volume
  - [ ] Pause / unpause
  - [ ] Set filters
  - [ ] Update voice state
  - [ ] `noReplace` query param
- [ ] `DELETE /v4/sessions/{sid}/players/{gid}` — destroy player

### REST — Session API
- [ ] `PATCH /v4/sessions/{sid}` — configure resuming

### REST — Info / Stats
- [ ] `GET /v4/info` — server info
- [ ] `GET /version` — version string
- [ ] `GET /v4/stats` — server stats

### REST — RoutePlanner API
- [ ] `GET /v4/routeplanner/status` — get status
- [ ] `POST /v4/routeplanner/free/address` — unmark address
- [ ] `POST /v4/routeplanner/free/all` — unmark all

### Filters
- [ ] `equalizer`
- [ ] `karaoke`
- [ ] `timescale`
- [ ] `tremolo`
- [ ] `vibrato`
- [ ] `rotation`
- [ ] `distortion`
- [ ] `channelMix`
- [ ] `lowPass`
- [ ] `pluginFilters`

### Error Handling
- [ ] Deserialize REST error responses
- [ ] Handle `?trace=true` debug mode
- [ ] Handle 204 No Content responses
- [ ] Handle connection drops gracefully

### Data Types (Rust Structs)
- [ ] `Track`
- [ ] `TrackInfo`
- [ ] `PlaylistInfo`
- [ ] `Player`
- [ ] `PlayerState`
- [ ] `VoiceState`
- [ ] `Filters` (and all sub-filter types)
- [ ] `Exception`
- [ ] `LoadResult` (tagged enum)
- [ ] `LavalinkMessage` (tagged enum for WS)
- [ ] `EventOp` (tagged enum for events)
- [ ] `TrackEndReason` (enum)
- [ ] `Severity` (enum)
- [ ] `Stats` / `Memory` / `Cpu` / `FrameStats`
- [ ] `LavalinkInfo` / `Version` / `Git` / `Plugin`
- [ ] `RoutePlannerStatus` / `Details` / `IpBlock` / `FailingAddress`
- [ ] `ErrorResponse`
