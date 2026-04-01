# Installing Wraith Browser

## Option 1: Pre-built Binaries

Download the latest release binary for your platform from the releases page (when available).

### Windows

```powershell
# Download the binary
# wraith-browser-X.Y.Z-windows-x86_64.exe

# Verify checksum
certutil -hashfile wraith-browser-0.1.0-windows-x86_64.exe SHA256
# Compare output against SHA256SUMS.txt

# Move to a directory on your PATH
move wraith-browser-0.1.0-windows-x86_64.exe C:\tools\wraith-browser.exe
```

### Linux

```bash
# Download the binary
# wraith-browser-X.Y.Z-linux-x86_64

# Verify checksum
sha256sum -c SHA256SUMS.txt

# Make executable and move to PATH
chmod +x wraith-browser-*-linux-x86_64
sudo mv wraith-browser-*-linux-x86_64 /usr/local/bin/wraith-browser
```

### macOS

macOS binaries must be built from source (see below). Cross-compilation from Windows to macOS is not supported.

---

## Option 2: Build from Source

### Prerequisites

- [Rust](https://rustup.rs/) 1.75 or later
- A C linker (MSVC on Windows, gcc/clang on Linux/macOS)

### Steps

```bash
git clone https://github.com/suhteevah/wraith-browser.git
cd wraith-browser
cargo build --release
```

The binary will be at `target/release/wraith-browser` (or `wraith-browser.exe` on Windows).

### Build release binaries for distribution

From the repo root:

```bash
# Bash (Git Bash on Windows, or native Linux/macOS shell)
bash scripts/build-release.sh

# PowerShell (Windows)
.\scripts\build-release.ps1
```

Binaries and checksums are written to the `dist/` directory.

#### Cross-compiling for Linux from Windows

The build script uses [cross](https://github.com/cross-rs/cross) for Linux builds:

```bash
# One-time setup
cargo install cross
# Ensure Docker Desktop is running

# Then just run the build script — it detects cross automatically
bash scripts/build-release.sh
```

---

## Option 3: Docker

A Dockerfile is provided in `deploy/`:

```bash
cd deploy
docker compose up --build
```

See `deploy/README.md` for configuration details.

---

## Post-install: Connect to Claude Code

```bash
claude mcp add wraith ./target/release/wraith-browser -- serve --transport stdio
```

## Environment Variables

| Variable | Purpose |
|----------|---------|
| `WRAITH_FLARESOLVERR` | URL for external challenge-handling proxy |
| `WRAITH_PROXY` | Primary HTTP/SOCKS5 proxy URL |
| `WRAITH_FALLBACK_PROXY` | Fallback proxy for IP-blocked sites |
| `ANTHROPIC_API_KEY` | Required for `browse_task` autonomous agent |
| `BRAVE_SEARCH_API_KEY` | Optional Brave Search provider |
| `TWOCAPTCHA_API_KEY` | Required for `browse_solve_captcha` CAPTCHA integration |

## Platform Notes

- **Windows**: Builds natively. The build scripts set `RUSTC_WRAPPER=""` to avoid sccache issues.
- **Linux x86_64**: Cross-compile from Windows using `cross` (Docker required), or build natively on a Linux machine.
- **macOS (Intel & Apple Silicon)**: Must be built on a Mac. Apple's toolchain cannot be used outside macOS.
