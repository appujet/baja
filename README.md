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

- [x] **YouTube**
- [x] **SoundCloud**
- [x] **Deezer**
- [x] **Qobuz**
- [x] **JioSaavn**
- [x] **Gaana**
- [x] **Bandcamp**
- [x] **MixCloud**
- [x] **Audiomack**
- [x] **Reddit**
- [x] **HTTP / Local**

### Mirroring
Resolution of metadata-only sources via secondary providers.

- [x] **Spotify**
- [x] **Apple Music**
- [x] **Tidal**
- [x] **Yandex**
- [x] **Shazam**
- [x] **Anghami**
- [x] **Pandora**
- [ ] **Last.fm**

### Utilities
- [x] **Text-to-Speech**

## Major Dependencies

Rustalink leverages a modern Rust ecosystem to provide high-performance audio processing:

- **[Tokio](https://tokio.rs/)**: High-performance asynchronous runtime.
- **[Axum](https://github.com/tokio-rs/axum)**: Erskine web framework for the control plane.
- **[Symphonia](https://github.com/pdeljanov/Symphonia)**: Pure Rust audio decoding and media demuxing.
- **[Reqwest](https://github.com/seanmonstar/reqwest)**: Reliable HTTP client for metadata and stream fetching.
- **[Davey](https://github.com/bongodevs/davey)**: Custom Discord DAVE protocol implementation.
- **[Audiopus](https://github.com/bongodevs/audiopus)**: High-performance Opus codec bindings.
- **[Prometheus](https://prometheus.io/)**: Real-time metrics and monitoring.


## Quick Start (Docker)

Docker is the recommended way to run Rustalink.

```bash
# 1. Pull the image
docker pull ghcr.io/bongodevs/rustalink:latest

# 2. Setup config
mkdir rustalink && cd rustalink
docker run --rm ghcr.io/bongodevs/rustalink:latest cat config.example.toml > config.toml

# 3. Running with Docker Compose
# Create a docker-compose.yml file:
services:
  rustalink:
    image: ghcr.io/bongodevs/rustalink:latest
    ports: ["2333:2333"]
    volumes: ["./config.toml:/app/config.toml", "./logs:/app/logs"]
    restart: unless-stopped
```

### Build Docker Image from Source

If you'd rather build the Docker image yourself from local source instead of pulling a pre-built image:

```bash
git clone https://github.com/bongodevs/rustalink.git
cd rustalink

# Build image from source (compiles Rust inside Docker — no local Rust toolchain needed)
docker build --target local -t rustalink:dev .

# Run it
docker run -p 2333:2333 -v ./config.toml:/app/config.toml rustalink:dev
```

> [!NOTE]
> The `--target local` flag triggers a full in-container Rust build. This takes longer than pulling the pre-built image but requires no local Rust installation.

For native installation (Windows, Linux, macOS), see the [Releases](https://github.com/bongodevs/rustalink/releases) page.

---

## Building from Source

### Requirements
- **Rust**: Latest stable version is required.

#### Linux (Ubuntu/Debian)
```bash
sudo apt-get update
sudo apt-get install -y build-essential cmake pkg-config libssl-dev clang
```

#### macOS
```bash
brew install cmake pkg-config
# Ensure Xcode Command Line Tools are installed:
xcode-select --install
```

#### Windows
- Install [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) (select "Desktop development with C++").
- Install [CMake](https://cmake.org/download/).

---

```bash
git clone https://github.com/bongodevs/rustalink.git
cd rustalink
cargo build --release
```

The compiled binary will be at `target/release/rustalink`.


## Credits & Inspiration

Rustalink is an independent reimplementation in Rust and does not copy source code from the following projects. We acknowledge their design and architectural influence:

- **[Lavalink](https://github.com/lavalink-devs/Lavalink)** *(MIT License)* — The original standalone audio node. Rustalink implements the Lavalink v4 protocol and draws inspiration from its player management, session handling, and event emission design.
