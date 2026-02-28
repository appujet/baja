<p align="center">
  <img src="https://pub-19903466d24c44f9a9d94c9a3b2f4932.r2.dev/rastalink.png" alt="Rustalink Logo" width="200" height="200">
</p>

<h1 align="center">Rustalink</h1>

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

<p align="center">
  <b>Rustalink</b> is a high-performance, standalone Discord audio sending node written in <b>Rust</b>.<br>
  Designed for efficiency, reliability, and modern features.
</p>

---

## Key Features

- 🚀 **High Performance**: Built with Rust for minimal overhead and maximum throughput.
- 🎵 **Extensive Source Support**: Native support for 15+ audio platforms.
- 🔄 **Smart Mirroring**: Automatically find audio for metadata-only sources (Spotify, Apple Music, etc.).
- 📺 **Advanced YouTube Support**: Toggle between multiple clients (WEB, ANDROID, IOS, TV) to bypass restrictions.
- 🐳 **Docker Ready**: One-command deployment with pre-configured environments.
- 🛠 **Highly Configurable**: Fine-tune every aspect of the server via `config.toml`.

---

## Supported Sources

Rustalink supports direct playback and **Mirroring**. Mirroring allows playback from metadata-only services by automatically finding the best audio match from your configured mirror providers.

| Source | Type | Search Prefix | Features |
| :--- | :---: | :--- | :--- |
| **YouTube** | Direct | `ytsearch:`, `ytmsearch:` | `ytrec:`, Lyrics |
| **SoundCloud** | Direct | `scsearch:` | - |
| **Spotify** | Mirror | `spsearch:` | `sprec:` |
| **Apple Music**| Mirror | `amsearch:` | - |
| **Deezer** | Hybrid | `dzsearch:`, `dzisrc:` | `dzrec:`, Lyrics |
| **Tidal** | Mirror | `tdsearch:` | `tdrec:` |
| **Qobuz** | Hybrid | `qbsearch:`, `qbisrc:` | `qbrec:` |
| **Bandcamp** | Direct | `bcsearch:` | - |
| **MixCloud** | Direct | `mcsearch:` | - |
| **JioSaavn** | Hybrid | `jssearch:` | `jsrec:` |
| **Gaana** | Hybrid | `gnsearch:` | - |
| **Yandex Music**| Hybrid | `ymsearch:` | `ymrec:`, Lyrics |
| **Audiomack** | Hybrid | `amksearch:` | - |
| **Anghami** | Mirror | `agsearch:` | - |
| **Shazam** | Mirror | `shsearch:` | - |
| **Pandora** | Mirror | `pdsearch:` | `pdrec:` |
| **Audius** | Direct | `ausearch:`, `audsearch:` | - |
| **HTTP / Local**| Direct | - | - |
| **Google TTS**| Direct | `gtts:`, `speak:` | - |
| **Flowery TTS**| Direct | `ftts:` | Override parameters via query `?voice=...` |
| **LazyPy TTS**| Direct | `lazypytts:`, `lazytts:` | Override via format `lazypytts:Service:Voice:Text` |

> [!TIP]
> **Hybrid** sources support direct playback if credentials are provided. Otherwise, they seamlessly fall back to mirroring.

### Text-to-Speech (TTS) Sources

Rustalink features three distinct TTS sources with unique routing parameters you can use via search prefixes directly:

1. **Google TTS** (`gtts:` or `speak:`)
   - Uses standard Google Translate voices based on the `config.toml` language code.
   - Example: `gtts:Hello World`

2. **Flowery TTS** (`ftts:`)
   - Capable of overriding your `config.toml` voice and speed settings on the fly utilizing URL query variables. 
   - Example: `ftts:Hello World?voice=Bria&speed=1.5`

3. **LazyPy TTS** (`lazypytts:` or `lazytts:`)
   - Connects to various web TTS APIs. You can define the custom service and voice utilizing colon separators.
   - Example: `lazypytts:Amazon:Matthew:Hello World`

### YouTube Playback Clients

Bypass restrictions by switching between specialized clients:

| Client Alias | Search | Resolve | Playback |
| :--- | :---: | :---: | :---: |
| `WEB` | ✅ | ✅ | ✅ |
| `MWEB` / `REMIX` / `MUSIC_WEB` / `WEB_REMIX` | ✅ | ✅ | ✅ |
| `ANDROID` | ✅ | ✅ | ✅ |
| `IOS` | ✅ | ✅ | ✅ |
| `TV` / `TVHTML5` | ✅ | ✅ | ✅ |
| `TV_CAST` / `TVHTML5_CAST` | ✅ | ✅ | ✅ |
| `TV_EMBEDDED` / `TVHTML5_EMBEDDED` / `TVHTML5_SIMPLY_EMBEDDED_PLAYER` | ✅ | ✅ | ✅ |
| `MUSIC_ANDROID` / `MUSIC` / `ANDROID_MUSIC` | ✅ | ✅ | ✅ |
| `ANDROID_VR` / `ANDROIDVR` | ✅ | ❌ | ✅ |
| `WEB_EMBEDDED` / `WEBEMBEDDED` | ✅ | ❌ | ✅ |
| `WEB_PARENT_TOOLS` / `WEBPARENTTOOLS` | ✅ | ✅ | ❌ |

---

## Quick Start (Docker)

Docker is the recommended way to run Rustalink.

```bash
# 1. Pull the image
docker pull ghcr.io/bongodevs/rustalink:latest

# 2. Setup config
mkdir rustalink && cd rustalink
docker run --rm ghcr.io/bongodevs/rustalink:latest cat config.default.toml > config.toml

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

---

## ❤️ Credits & Inspiration

- **[Lavalink](https://github.com/lavalink-devs/Lavalink)** - The original standalone audio node.
- **[NodeLink](https://github.com/PerformanC/NodeLink)** - Lightweight Lavalink alternative.

---

## 📄 License

Rustalink is published under the **Apache License 2.0**.  
See the [LICENSE](https://github.com/bongodevs/Rustalink/blob/HEAD/LICENSE) file for more details.