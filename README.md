<p align="center">
  <img src="https://pub-19903466d24c44f9a9d94c9a3b2f4932.r2.dev/rastalink.svg" alt="Rustalink Logo" width="300" height="300">
</p>

<h1 align="center">Rustalink</h1>

<p align="center">
  Development-first, high-performance audio node for Discord.
</p>

<p align="center">
  <a href="https://github.com/bongodevs/Rustalink/releases"><img src="https://img.shields.io/github/v/release/bongodevs/Rustalink?style=for-the-badge&color=orange&logo=github" alt="Release"></a>
  <a href="https://github.com/bongodevs/Rustalink/actions"><img src="https://img.shields.io/github/actions/workflow/status/bongodevs/Rustalink/release.yml?style=for-the-badge&logo=githubactions&logoColor=white" alt="Build Status"></a>
  <a href="https://github.com/bongodevs/Rustalink/blob/HEAD/LICENSE"><img src="https://img.shields.io/github/license/bongodevs/Rustalink?style=for-the-badge&color=blue" alt="License"></a>
  <br>
  <img src="https://img.shields.io/badge/Language-Rust-orange?style=for-the-badge&logo=rust" alt="Language">
  <img src="https://img.shields.io/badge/Platform-Linux%20%7C%20Windows%20%7C%20macOS-lightgrey?style=for-the-badge" alt="Platform">
  <a href="https://github.com/bongodevs/Rustalink/stargazers"><img src="https://img.shields.io/github/stars/bongodevs/Rustalink?style=for-the-badge&color=yellow&logo=github" alt="Stars"></a>
</p>

---

Rustalink is a standalone audio sending node optimized for modern Discord bots. It provides a robust, low-latency bridge between your bot and multiple audio providers, following the Lavalink v4 specification for seamless integration.

## Features

Our core engine is built for efficiency, allowing thousands of concurrent streams with minimal resource consumption.

- **Infrastructure**
    - [x] Asynchronous, non-blocking I/O
    - [x] Sub-millisecond player precision
    - [x] State-persistence for session recovery
    - [x] Native cross-platform support
    - [x] Real-time hardware-accelerated filters


## Supported Platforms

Rustalink distinguishes between direct native playback and intelligent mirroring to ensure maximum availability.

### Native
Direct stream extraction and resolution.

- [x] **YouTube**: Full playback, search, and lyrics support.
- [x] **SoundCloud**: High-fidelity direct streaming.
- [x] **Deezer**: Native search and track resolution.
- [x] **Qobuz**: Native search and track resolution.
- [x] **JioSaavn**: Native search and track resolution.
- [x] **Gaana**: Native search and track resolution.
- [x] **Bandcamp**: Native search and track resolution.
- [x] **MixCloud**: Native search and track resolution.
- [x] **Audiomack**: Native search and track resolution.
- [x] **Reddit**: Url resolution and audio extraction.
- [x] **HTTP / Local**: Direct file and remote URL streaming.

### Mirroring
Resolution of metadata-only sources via secondary providers.

- [x] **Spotify**: Advanced matching via ISRC.
- [x] **Apple Music**: Comprehensive search-based resolution.
- [x] **Tidal**: Specialty provider mirroring (playback support but need a Tidal-HIFI subscription and refresh token) [Inspired from](https://github.com/yaronzz/Tidal-Media-Downloader).
- [x] **Yandex**: Specialty provider mirroring (playback support but need access token).
- [x] **Shazam**: Discovery-focused metadata resolution.
- [x] **Anghami**: Discovery-focused metadata resolution.
- [x] **Pandora**: Discovery-focused metadata resolution.

### Utilities
- [x] **Text-to-Speech**: Integrated Google and Flowery TTS.

## Major Dependencies

Rustalink leverages a modern Rust ecosystem to provide high-performance audio processing:

- **[Tokio](https://tokio.rs/)**: High-performance asynchronous runtime.
- **[Axum](https://github.com/tokio-rs/axum)**: Erskine web framework for the control plane.
- **[Symphonia](https://github.com/pdeljanov/Symphonia)**: Pure Rust audio decoding and media demuxing.
- **[Reqwest](https://github.com/seanmonstar/reqwest)**: Reliable HTTP client for metadata and stream fetching.
- **[Davey](https://github.com/bongodevs/davey)**: Custom Discord DAVE protocol implementation.
- **[Audiopus](https://github.com/bongodevs/audiopus)**: High-performance Opus codec bindings.
- **[Prometheus](https://prometheus.io/)**: Real-time metrics and monitoring.

## Getting Started

### Quick Deployment (Docker)
```bash
docker run -d \
  --name rustalink \
  -p 2333:2333 \
  -v $(pwd)/config.toml:/app/config.toml \
  --restart unless-stopped \
  ghcr.io/bongodevs/rustalink:latest
```


## Credits & Inspiration

Rustalink is an independent reimplementation in Rust and does not copy source code from the following projects. We acknowledge their design and architectural influence:

- **[Lavalink](https://github.com/lavalink-devs/Lavalink)** *(MIT License)* — The original standalone audio node. Rustalink implements the Lavalink v4 protocol and draws inspiration from its player management, session handling, and event emission design.
