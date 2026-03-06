<p align="center">
  <img src="https://pub-19903466d24c44f9a9d94c9a3b2f4932.r2.dev/rastalink.svg" alt="Rustalink Logo" width="160" height="160">
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

## Capabilities

Our core engine is built for efficiency, allowing thousands of concurrent streams with minimal resource consumption.

- **Infrastructure**
    - [x] Asynchronous, non-blocking I/O
    - [x] Sub-millisecond player precision
    - [x] State-persistence for session recovery
    - [x] Native cross-platform support

- **Audio Engine**
    - [x] Real-time hardware-accelerated filters
    - [x] Advanced client rotation for rate-limit bypass
    - [x] Smart metadata mirroring
    - [x] Comprehensive lyrics resolution

## Supported Platforms

Rustalink distinguishes between direct native playback and intelligent mirroring to ensure maximum availability.

### Native Integration
Direct stream extraction and resolution.

- [x] **YouTube**: Full playback, search, and lyrics support.
- [x] **SoundCloud**: High-fidelity direct streaming.
- [x] **Deezer**: Native search and track resolution.
- [x] **Qobuz / JioSaavn / Gaana**: Regional provider support.
- [x] **Bandcamp / MixCloud / Audiomack**: Creative platform integration.
- [x] **HTTP / Local**: Direct file and remote URL streaming.

### Intelligent Mirroring
Resolution of metadata-only sources via secondary providers.

- [x] **Spotify**: Advanced matching via ISRC.
- [x] **Apple Music**: Comprehensive search-based resolution.
- [x] **Tidal / Yandex**: Specialty provider mirroring.
- [x] **Shazam / Anghami / Pandora**: Discovery-focused metadata resolution.

### Utilities
- [x] **Text-to-Speech**: Integrated Google and Flowery TTS.

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

### Documentation & API
Detailed guides, TOML configuration references, and full API documentation are available at:

**[soon](https://soon.gg)**

## Credits & Inspiration

Rustalink is an independent reimplementation in Rust and does not copy source code from the following projects. We acknowledge their design and architectural influence:

- **[Lavalink](https://github.com/lavalink-devs/Lavalink)** *(MIT License)* — The original standalone audio node. Rustalink implements the Lavalink v4 protocol and draws inspiration from its player management, session handling, and event emission design.
