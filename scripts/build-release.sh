#!/usr/bin/env bash
# Build wraith-browser release binaries for multiple platforms.
# Run from repo root: bash scripts/build-release.sh
#
# Prerequisites:
#   - Rust toolchain (rustup + cargo)
#   - rustup target add x86_64-unknown-linux-gnu  (for Linux builds)
#   - cargo install cross  (for Linux cross-compilation from Windows; requires Docker)
#
# macOS cross-compilation from Windows is effectively impossible without a real Mac.
# Apple's linker and SDK are not redistributable, and no reliable open-source
# toolchain exists for Windows-to-macOS compilation. If you need macOS binaries,
# build on an actual Mac or use a macOS VM/CI service.
#
# Outputs to: dist/

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

# ---------- version ----------
VERSION=$(cargo metadata --format-version=1 --no-deps 2>/dev/null \
  | grep -o '"version":"[^"]*"' | head -1 | cut -d'"' -f4) || true
VERSION="${VERSION:-0.1.0}"

DIST="dist"
rm -rf "$DIST"
mkdir -p "$DIST"

BUILT=()
FAILED=()

echo "========================================"
echo "  wraith-browser release build v${VERSION}"
echo "========================================"
echo ""

# ---------- helper ----------
sha256_file() {
  if command -v sha256sum &>/dev/null; then
    sha256sum "$1"
  elif command -v shasum &>/dev/null; then
    shasum -a 256 "$1"
  elif command -v certutil &>/dev/null; then
    # Windows fallback (certutil output needs reformatting)
    hash=$(certutil -hashfile "$1" SHA256 | sed -n '2p' | tr -d ' ')
    echo "$hash  $1"
  else
    echo "(no sha256 tool found)  $1"
  fi
}

# ==========================================================
#  1. Windows x86_64 — native build (always works)
# ==========================================================
echo ">>> Building for Windows x86_64 (native)..."
RUSTC_WRAPPER="" cargo build --release 2>&1

WIN_BIN="target/release/wraith-browser.exe"
if [ -f "$WIN_BIN" ]; then
  OUT_NAME="wraith-browser-${VERSION}-windows-x86_64.exe"
  cp "$WIN_BIN" "$DIST/$OUT_NAME"
  BUILT+=("$OUT_NAME")
  echo "    OK: $OUT_NAME"
else
  FAILED+=("windows-x86_64")
  echo "    FAILED: Windows binary not found at $WIN_BIN"
fi
echo ""

# ==========================================================
#  2. Linux x86_64 — cross-compile via `cross` (requires Docker)
# ==========================================================
if command -v cross &>/dev/null; then
  echo ">>> Building for Linux x86_64 (via cross)..."
  if RUSTC_WRAPPER="" cross build --release --target x86_64-unknown-linux-gnu 2>&1; then
    LINUX_BIN="target/x86_64-unknown-linux-gnu/release/wraith-browser"
    if [ -f "$LINUX_BIN" ]; then
      OUT_NAME="wraith-browser-${VERSION}-linux-x86_64"
      cp "$LINUX_BIN" "$DIST/$OUT_NAME"
      BUILT+=("$OUT_NAME")
      echo "    OK: $OUT_NAME"
    else
      FAILED+=("linux-x86_64")
      echo "    FAILED: Linux binary not found at $LINUX_BIN"
    fi
  else
    FAILED+=("linux-x86_64")
    echo "    FAILED: cross build returned an error"
  fi
else
  echo ">>> SKIPPED: Linux x86_64 — 'cross' not installed."
  echo "   Install with: cargo install cross"
  echo "   Requires Docker Desktop running."
  FAILED+=("linux-x86_64 (skipped)")
fi
echo ""

# ==========================================================
#  3. macOS — NOT supported from Windows
# ==========================================================
# macOS cross-compilation from Windows is not feasible.
# Apple's toolchain (xcrun, ld64, macOS SDK) cannot be legally or practically
# used outside macOS. If you need macOS binaries:
#   - Build on a Mac: cargo build --release
#   - Use a macOS CI runner (GitHub Actions, Cirrus CI, etc.)
#   - Universal binary: cargo build --release --target aarch64-apple-darwin
#                       cargo build --release --target x86_64-apple-darwin
#                       lipo -create -output wraith-browser-universal <both binaries>
echo ">>> SKIPPED: macOS — cross-compilation from Windows is not supported."
echo "   Build on a Mac with: cargo build --release"
echo ""

# ==========================================================
#  4. Generate SHA256 checksums
# ==========================================================
echo ">>> Generating checksums..."
CHECKSUM_FILE="$DIST/SHA256SUMS.txt"
: > "$CHECKSUM_FILE"

for f in "$DIST"/wraith-browser-*; do
  [ -f "$f" ] || continue
  sha256_file "$f" >> "$CHECKSUM_FILE"
done

if [ -s "$CHECKSUM_FILE" ]; then
  echo "    Wrote $CHECKSUM_FILE"
else
  echo "    No binaries to checksum."
fi
echo ""

# ==========================================================
#  Summary
# ==========================================================
echo "========================================"
echo "  Build Summary"
echo "========================================"
echo ""
echo "  Version:  $VERSION"
echo "  Output:   $DIST/"
echo ""

if [ ${#BUILT[@]} -gt 0 ]; then
  echo "  Built successfully:"
  for b in "${BUILT[@]}"; do
    SIZE=$(wc -c < "$DIST/$b" 2>/dev/null | tr -d ' ')
    SIZE_MB=$(awk "BEGIN {printf \"%.1f\", $SIZE/1048576}")
    echo "    - $b  (${SIZE_MB} MB)"
  done
else
  echo "  No binaries were built successfully."
fi
echo ""

if [ ${#FAILED[@]} -gt 0 ]; then
  echo "  Failed / Skipped:"
  for f in "${FAILED[@]}"; do
    echo "    - $f"
  done
fi
echo ""
echo "Done."
