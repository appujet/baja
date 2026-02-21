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

*Last updated: 2026-02-22*
