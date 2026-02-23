# rustalink 🦈

**rustalink** is a high-performance, v4-compatible Lavalink server implementation written in **Rust**. Built with efficiency and modern features in mind, it aims to provide a robust alternative for Discord bot audio providers.

## 🚀 Progress Tracking

The following table outlines the current implementation status of various features:

### Core Infrastructure
| Feature | Status | Description |
| :--- | :---: | :--- |
| **Lavalink v4 REST API** | ✅ | Full compatibility with v4 endpoints |
| **WebSocket Interface** | ✅ | Event dispatching and real-time stats |
| **Session Management** | ✅ | Session creation, discovery, and cleanup |
| **Resumable Sessions** | ✅ | Connection persistence across restarts/disconnects |
| **Discord Gateway** | ✅ | Robust voice state and server update handling |
| **Discord UDP** | ✅ | Direct audio data transmission to Discord |
| **Discord DAVE** | ✅ | Support for E2EE (End-to-End Encryption) |

### Audio Engine
| Feature | Status | Description |
| :--- | :---: | :--- |
| **Symphonia Decoding** | ✅ | Hardware-accelerated audio decoding |
| **PCM Resampling** | ✅ | High-quality resampling to 48kHz |
| **Audio Mixing** | ✅ | Multi-track mixing support |
| **Opus Encoding** | ✅ | Low-latency encoding for Discord |
| **Audio Filters** | ✅ | Implementation of EQ, Karaoke, Timescale, etc. |
| **Seeking** | ✅ | Support for seeking within tracks |


### Audio Sources
| Source | Status | Description |
| :--- | :---: | :--- |
| **HTTP / HTTPS** | ✅ | Direct streaming from web URLs |
| **Local** | ✅ | Direct streaming from local files |
| **YouTube** | ✅ | Integration with `TV` and `IOS` client are supported for playback and (`sabr` streaming is under development) |
| **Spotify** | ✅ | Metadata resolution and full mirror playback support |
| **JioSaavn** | ✅ | Metadata resolution and full playback support |
| **Amazon Music** | ❌ | Planned implementation |
| **Apple Music** | ✅ | Implementation |
| **Anghami** | ✅ | Metadata resolution with full mirror playback support (Protobuf-encoded response handling) |
| **Audiomack** | ✅ | Implementation |
| **Audius** | ✅ | Implementation |
| **Bandcamp** | ✅ | Implementation |
| **Bilibili** | ❌ | Planned implementation |
| **Deezer** | ✅ | Implementation |
| **Gaana** | ✅ | Implementation |
| **Kwai** | ❌ | Planned implementation |
| **Last.fm** | ❌ | Planned implementation |
| **MixCloud** | ✅ | Implementation |
| **Pandora** | ✅ | Implementation |
| **Qobuz** | ✅ | Implementation |
| **Reddit** | ❌ | Planned implementation |
| **Shazam** | ✅ | Implementation |
| **SoundCloud** | ✅ | Integration with progressive and HLS streams |
| **Tidal** | ✅ | Implementation |
| **Twitch** | ❌ | Planned implementation |
| **Vimeo** | ❌ | Planned implementation |
| **VK Music** | ❌ | Planned implementation |
| **Yandex Music** | ❌ | Planned implementation |

---

## 📖 Getting Started

Ready to use **rustalink**? Check out our comprehensive setup guide:

👉 **[Setup & Usage Guide (Docker, Windows, Linux, macOS)](./guide.md)**

---

## 🛠️ Performance
rustalink is designed to be extremely lightweight, leveraging Rust's zero-cost abstractions and asynchronous runtime (**Tokio**) to handle hundreds of concurrent streams with minimal CPU and memory footprint.

## ⚙️ Requirements

### 🛠️ Build Requirements
If you are building from source, you need the following installed on your system:

- **Rust**: Latest stable version (Edition 2024).
- **C/C++ Toolchain**: `gcc`, `g++`, `make`.
- **CMake**: Required for building bundled C dependencies (`opus`).
- **CMake**: Used for building some C/C++ dependencies (like Opus).
- **Clang/LLVM**: Required for `bindgen` (e.g., `libclang-dev`).
- **Pkg-config**: To locate system libraries.

#### Platform Specific Install Commands:
- **Ubuntu/Debian**:
  ```bash
  sudo apt-get update
  sudo apt-get install -y cmake pkg-config libclang-dev clang gcc g++ make perl
  ```
- **Arch Linux**:
  ```bash
  sudo pacman -S cmake pkgconf clang gcc make perl
  ```
- **macOS**:
  ```bash
  brew install cmake pkg-config
  ```
- **Windows**:
  - [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) with C++ workload.
  - [LLVM/Clang](https://releases.llvm.org/download.html) (add to PATH).

### 🏃 Runtime Requirements
- **Docker** (Optional, recommended): For running the pre-built multi-arch image.
- **OpenSSL**: Ensure system certificates are up to date (usually present by default).
- **Visual C++ Redistributable**: (Windows only) Required for native binaries.




## Format Code

```bash
rustup run nightly cargo fmt
```