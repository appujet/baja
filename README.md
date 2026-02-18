# rustalink 🦈

**rustalink** is a high-performance, v4-compatible Lavalink server implementation written in **Rust**. Built with efficiency and modern features in mind, it aims to provide a robust alternative for Discord bot audio providers.

## 🚀 Progress Tracking

The following table outlines the current implementation status of various features:

### Core Infrastructure
| Feature | Status | Description |
| :--- | :---: | :--- |
| **Lavalink v4 REST API** | 🚧 | Full compatibility with v4 endpoints |
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
| **Audio Filters** | 🚧 | Implementation of EQ, Karaoke, Timescale, etc. |

### Audio Sources
| Source | Status | Description |
| :--- | :---: | :--- |
| **HTTP / HTTPS** | ✅ | Direct streaming from web URLs |
| **YouTube** | 🚧 | Integration with `rustypipe` / `yt-dlp` |
| **Spotify** | 🚧 | Metadata resolution and playback |
| **SoundCloud** | ❌ | Planned implementation |
| **Deezer** | ❌ | Planned implementation |

---

## 🛠️ Performance
rustalink is designed to be extremely lightweight, leveraging Rust's zero-cost abstractions and asynchronous runtime (**Tokio**) to handle hundreds of concurrent streams with minimal CPU and memory footprint.

## ⚙️ Requirements
- **Rust** (Edition 2024)
- **C Compiler** (for `audiopus` / `opus` dependencies)
- **Discord Bot Token**
