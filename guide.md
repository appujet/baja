# Rustalink Implementation & Operational Guide

This guide provides step-by-step instructions for running **Rustalink** using Docker and as a native binary on various operating systems.

---

## 🚀 Running with Docker (Recommended)

Docker is the easiest way to run Rustalink as it handles all dependencies and provides a consistent environment.

### 1. Pull the Image
Pull the latest multi-arch image from the GitHub Container Registry (GHCR):
```bash
docker pull ghcr.io/${GITHUB_REPOSITORY}/rustalink:latest
```
*(Replace `${GITHUB_REPOSITORY}` with your actual repository path, e.g., `appujet/baja`)*

### 2. Prepare Configuration
Create a directory for your configuration and logs:
```bash
mkdir rustalink-data
cp config.default.toml rustalink-data/config.toml
```
Edit `rustalink-data/config.toml` to your liking.

### 3. Run the Container
```bash
docker run -d \
  --name rustalink \
  -p 2333:2333 \
  -v $(pwd)/rustalink-data/config.toml:/app/config.toml \
  -v $(pwd)/rustalink-data/logs:/app/logs \
  --restart unless-stopped \
  ghcr.io/${GITHUB_REPOSITORY}/rustalink:latest
```

### 4. Optional: Docker Compose
Create a `docker-compose.yml`:
```yaml
services:
  rustalink:
    image: ghcr.io/${GITHUB_REPOSITORY}/rustalink:latest
    ports:
      - "2333:2333"
    volumes:
      - ./config.toml:/app/config.toml
      - ./logs:/app/logs
    restart: unless-stopped
```
Run with: `docker compose up -d`

---

## 💻 Running Native (Non-Docker)

If you prefer to run directly on your OS, download the appropriate binary from the [GitHub Releases](https://github.com/${GITHUB_REPOSITORY}/releases) page or build from source.

### Setup Prerequisites
All platforms require:
1. `config.toml` in the same directory as the binary.
2. Port **2333** to be open in your firewall.

#### 🪟 Windows Setup
1. Download `rustalink-x86_64-pc-windows-msvc.exe`.
2. Install the [Visual C++ Redistributable](https://aka.ms/vs/17/release/vc_redist.x64.exe) if not already present.
3. Place `config.toml` next to the `.exe`.
4. Run the executable via Command Prompt or PowerShell:
   ```powershell
   .\rustalink.exe
   ```

#### 🏔️ Linux (Arch / Ubuntu) Setup
1. Download `rustalink-x86_64-unknown-linux-musl` (Static binary).
2. Install system dependencies (for building or if dynamic):
   *   **Arch**: `sudo pacman -S openssl cmake gcc pkg-config`
   *   **Ubuntu/Debian**: `sudo apt install openssl cmake build-essential pkg-config`
3. Make the binary executable:
   ```bash
   chmod +x rustalink-x86_64-unknown-linux-musl
   ```
4. Run:
   ```bash
   ./rustalink-x86_64-unknown-linux-musl
   ```

#### 🍎 macOS Setup
1. Download the correct binary for your CPU:
   *   Intel: `rustalink-x86_64-apple-darwin`
   *   M1/M2/M3: `rustalink-aarch64-apple-darwin`
2. Remove any "unidentified developer" quarantine (if blocked):
   ```bash
   xattr -d com.apple.quarantine rustalink-*-apple-darwin
   ```
3. Make executable and run:
   ```bash
   chmod +x rustalink-*-apple-darwin
   ./rustalink-*-apple-darwin
   ```

---

## ⚙️ Configuration Notes

*   **Default Port**: 2333
*   **Default Password**: `youshallnotpass`
*   **Log Level**: Modified via the `[logging]` section in `config.toml`.
*   **Sources**: Enable or disable music sources (YouTube, Spotify, etc.) in the `[sources]` section.

If you encounter issues, check the `logs/` directory for detailed error messages.
