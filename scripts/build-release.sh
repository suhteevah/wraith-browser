#!/usr/bin/env bash
# build-release.sh — cross-compile wraith-browser (open-source CLI binary)
# for all release targets and stage tarballs/zips under dist/${VERSION}/.
#
# Usage:
#   bash scripts/build-release.sh v0.1.0
#   bash scripts/build-release.sh v0.1.0 --force        # nuke non-empty dist/$V/
#   bash scripts/build-release.sh v0.1.0 --publish      # gh release create after
#   bash scripts/build-release.sh v0.1.0 --skip-windows # debug
#
# Builds locally on Kokonoe — NO GitHub Actions (banned per CLAUDE.md).
# macOS targets are emitted as `ssh imac` commands for the operator to run
# manually; drop the resulting tarballs into dist/${VERSION}/ before --publish.
#
# Targets (see scripts/release-targets.md for rationale):
#   x86_64-unknown-linux-gnu     Docker (rust:1.88-slim-bookworm)         [parallel]
#   aarch64-unknown-linux-gnu    Docker buildx --platform linux/arm64     [parallel]
#   x86_64-pc-windows-msvc       Native cargo on Kokonoe                  [parallel]
#   x86_64-apple-darwin          imac via SSH (operator runs)             [manual]
#   aarch64-apple-darwin         imac via SSH (operator runs)             [manual]

set -euo pipefail

# ---------------------------------------------------------------------
# Args
# ---------------------------------------------------------------------
VERSION="${1:-}"
shift || true

if [[ -z "$VERSION" ]]; then
  echo "ERROR: version required, e.g. bash scripts/build-release.sh v0.1.0" >&2
  exit 2
fi

FORCE=0
PUBLISH=0
SKIP_WINDOWS=0
SKIP_LINUX=0
for arg in "$@"; do
  case "$arg" in
    --force)         FORCE=1 ;;
    --publish)       PUBLISH=1 ;;
    --skip-windows)  SKIP_WINDOWS=1 ;;
    --skip-linux)    SKIP_LINUX=1 ;;
    *) echo "ERROR: unknown flag: $arg" >&2; exit 2 ;;
  esac
done

# Strip leading 'v' for filename embedding (we keep raw $VERSION for the gh tag)
V_BARE="${VERSION#v}"

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

DIST_ROOT="${REPO_ROOT}/dist"
DIST="${DIST_ROOT}/${VERSION}"
CACHE_ROOT="${DIST_ROOT}/.cache"
LOG_DIR="${DIST}/_logs"

log()  { echo "[build-release] $*" >&2; }
fail() { echo "[build-release] FATAL: $*" >&2; exit 1; }

# ---------------------------------------------------------------------
# 1. Sanity
# ---------------------------------------------------------------------
log "version: ${VERSION} (bare ${V_BARE})"
log "repo:    ${REPO_ROOT}"

# Working tree clean?
if [[ -n "$(git status --porcelain)" ]]; then
  log "WARNING: working tree is not clean:"
  git status --short >&2
  read -r -p "Continue anyway? [y/N] " ans
  [[ "$ans" =~ ^[Yy]$ ]] || fail "aborted (dirty tree)"
fi

# Cargo.toml workspace.package.version must match
TOML_VERSION="$(grep -E '^version\s*=' Cargo.toml | head -1 | sed -E 's/version\s*=\s*"([^"]+)".*/\1/')"
if [[ "$TOML_VERSION" != "$V_BARE" ]]; then
  log "WARNING: Cargo.toml workspace.package.version=$TOML_VERSION but you asked for $V_BARE"
  read -r -p "Continue anyway? [y/N] " ans
  [[ "$ans" =~ ^[Yy]$ ]] || fail "aborted (version mismatch)"
fi

# dist/$VERSION/ refusal
if [[ -d "$DIST" ]] && [[ -n "$(ls -A "$DIST" 2>/dev/null || true)" ]]; then
  if [[ "$FORCE" -eq 1 ]]; then
    log "--force: wiping ${DIST}"
    rm -rf "$DIST"
  else
    fail "${DIST} is non-empty. Pass --force to wipe it."
  fi
fi
mkdir -p "$DIST" "$LOG_DIR" "$CACHE_ROOT"

# Required tools
command -v cargo  >/dev/null  || fail "cargo not on PATH"
if [[ "$SKIP_LINUX" -eq 0 ]]; then
  command -v docker >/dev/null  || fail "docker not on PATH (need Docker Desktop running)"
  docker info >/dev/null 2>&1   || fail "docker daemon not responding"
fi
if [[ "$PUBLISH" -eq 1 ]]; then
  command -v gh >/dev/null      || fail "gh CLI not on PATH (--publish requested)"
  [[ -f "$REPO_ROOT/CHANGELOG_FRAGMENT.md" ]] \
    || fail "CHANGELOG_FRAGMENT.md missing (--publish requires it)"
fi

# ---------------------------------------------------------------------
# 2. Linux builds (Docker, in parallel)
# ---------------------------------------------------------------------
RUST_IMAGE="rust:1.88-slim-bookworm"

# Ensure builder image is local before parallel jobs race on it.
if [[ "$SKIP_LINUX" -eq 0 ]]; then
  log "pulling ${RUST_IMAGE} (one-time)"
  docker pull "$RUST_IMAGE" >>"$LOG_DIR/docker-pull.log" 2>&1 \
    || fail "docker pull failed (see $LOG_DIR/docker-pull.log)"
fi

build_linux() {
  local triple="$1"
  local platform="$2"
  local short="$3"   # x86_64-linux | aarch64-linux
  local out_archive="wraith-browser-${V_BARE}-${short}.tar.gz"

  local target_cache="${CACHE_ROOT}/target-${triple}"
  local registry_cache="${CACHE_ROOT}/cargo-registry"
  mkdir -p "$target_cache" "$registry_cache"

  local logf="${LOG_DIR}/${short}.log"
  log "[${short}] building via docker (${platform})"

  # Bind-mount everything; install rustup target inside the container, build,
  # then strip + copy the binary out. We do NOT COPY the workspace (~1GB).
  # The container needs cmake/perl for boring-sys, pkg-config for openssl-sys.
  docker run --rm \
    --platform "${platform}" \
    -v "${REPO_ROOT}:/build:ro" \
    -v "${target_cache}:/target" \
    -v "${registry_cache}:/usr/local/cargo/registry" \
    -e CARGO_TARGET_DIR=/target \
    -w /build \
    "$RUST_IMAGE" \
    bash -c '
      set -euo pipefail
      apt-get update >/dev/null
      apt-get install -y --no-install-recommends \
        build-essential cmake perl pkg-config libssl-dev ca-certificates \
        >/dev/null
      rustup target add '"$triple"' >/dev/null
      cargo build --release \
        --target '"$triple"' \
        --bin wraith-browser \
        --manifest-path /build/Cargo.toml
      strip /target/'"$triple"'/release/wraith-browser || true
      mkdir -p /target/_stage
      cp /target/'"$triple"'/release/wraith-browser /target/_stage/wraith-browser
    ' >"$logf" 2>&1 \
    || { echo "[${short}] FAILED — see $logf" >&2; return 1; }

  local stage_bin="${target_cache}/_stage/wraith-browser"
  [[ -f "$stage_bin" ]] || { echo "[${short}] binary missing after build" >&2; return 1; }

  # Pack tar.gz with a short top-level dir
  local pack_dir="${LOG_DIR}/_pack-${short}"
  rm -rf "$pack_dir"
  mkdir -p "${pack_dir}/wraith-browser-${V_BARE}-${short}"
  cp "$stage_bin" "${pack_dir}/wraith-browser-${V_BARE}-${short}/wraith-browser"
  cp "${REPO_ROOT}/LICENSE"   "${pack_dir}/wraith-browser-${V_BARE}-${short}/" 2>/dev/null || true
  cp "${REPO_ROOT}/README.md" "${pack_dir}/wraith-browser-${V_BARE}-${short}/" 2>/dev/null || true

  ( cd "$pack_dir" && tar -czf "${DIST}/${out_archive}" "wraith-browser-${V_BARE}-${short}" )
  log "[${short}] OK -> ${DIST}/${out_archive}"
}

linux_pids=()
if [[ "$SKIP_LINUX" -eq 0 ]]; then
  ( build_linux "x86_64-unknown-linux-gnu"  "linux/amd64" "x86_64-linux"  ) &
  linux_pids+=($!)

  ( build_linux "aarch64-unknown-linux-gnu" "linux/arm64" "aarch64-linux" ) &
  linux_pids+=($!)
fi

# ---------------------------------------------------------------------
# 3. Native Windows build (in parallel with Docker)
# ---------------------------------------------------------------------
build_windows() {
  local short="x86_64-windows"
  local out_archive="wraith-browser-${V_BARE}-${short}.zip"
  local logf="${LOG_DIR}/${short}.log"

  log "[${short}] building native (cargo --release)"
  ( cd "$REPO_ROOT"
    RUSTC_WRAPPER="" cargo build --release --bin wraith-browser
  ) >"$logf" 2>&1 \
    || { echo "[${short}] FAILED — see $logf" >&2; return 1; }

  local exe="${REPO_ROOT}/target/release/wraith-browser.exe"
  [[ -f "$exe" ]] || { echo "[${short}] exe missing after build" >&2; return 1; }

  local pack_dir="${LOG_DIR}/_pack-${short}"
  rm -rf "$pack_dir"
  mkdir -p "${pack_dir}/wraith-browser-${V_BARE}-${short}"
  cp "$exe" "${pack_dir}/wraith-browser-${V_BARE}-${short}/wraith-browser.exe"
  cp "${REPO_ROOT}/LICENSE"   "${pack_dir}/wraith-browser-${V_BARE}-${short}/" 2>/dev/null || true
  cp "${REPO_ROOT}/README.md" "${pack_dir}/wraith-browser-${V_BARE}-${short}/" 2>/dev/null || true

  # Use PowerShell Compress-Archive for portability (Git Bash zip not always present)
  if command -v zip >/dev/null; then
    ( cd "$pack_dir" && zip -qr "${DIST}/${out_archive}" "wraith-browser-${V_BARE}-${short}" )
  else
    powershell.exe -NoProfile -Command \
      "Compress-Archive -Path '$(cygpath -w "$pack_dir")\\wraith-browser-${V_BARE}-${short}' -DestinationPath '$(cygpath -w "${DIST}/${out_archive}")' -Force" \
      >>"$logf" 2>&1
  fi
  log "[${short}] OK -> ${DIST}/${out_archive}"
}

win_pid=""
if [[ "$SKIP_WINDOWS" -eq 0 ]]; then
  ( build_windows ) &
  win_pid=$!
fi

# ---------------------------------------------------------------------
# 4. Wait on parallel jobs, collect failures
# ---------------------------------------------------------------------
declare -a FAILED_TARGETS=()

for pid in "${linux_pids[@]}"; do
  wait "$pid" || FAILED_TARGETS+=("linux-pid-$pid")
done
if [[ -n "$win_pid" ]]; then
  wait "$win_pid" || FAILED_TARGETS+=("x86_64-windows")
fi

# ---------------------------------------------------------------------
# 5. macOS — emit ssh imac commands
# ---------------------------------------------------------------------
MAC_INSTRUCTIONS="${DIST}/_macos-build.txt"
cat >"$MAC_INSTRUCTIONS" <<EOF
# macOS builds — run on imac (per ~/.ssh/config 'Host imac')
# Prereq on imac: rustup default 1.88 (or newer); rustup target add x86_64-apple-darwin
#
# Then run BOTH of these from Kokonoe (Git Bash):

ssh imac '
  set -e
  cd ~/src/wraith-browser
  git fetch --all --prune
  git checkout ${VERSION} 2>/dev/null || git checkout main
  git pull --ff-only

  # Native arm64 (M-series)
  cargo build --release --bin wraith-browser --target aarch64-apple-darwin
  strip target/aarch64-apple-darwin/release/wraith-browser
  mkdir -p /tmp/wb-pack/wraith-browser-${V_BARE}-aarch64-macos
  cp target/aarch64-apple-darwin/release/wraith-browser /tmp/wb-pack/wraith-browser-${V_BARE}-aarch64-macos/
  cp LICENSE README.md /tmp/wb-pack/wraith-browser-${V_BARE}-aarch64-macos/ 2>/dev/null || true
  ( cd /tmp/wb-pack && tar -czf ~/wraith-browser-${V_BARE}-aarch64-macos.tar.gz wraith-browser-${V_BARE}-aarch64-macos )

  # Cross to x86_64
  rustup target add x86_64-apple-darwin >/dev/null
  cargo build --release --bin wraith-browser --target x86_64-apple-darwin
  strip target/x86_64-apple-darwin/release/wraith-browser
  rm -rf /tmp/wb-pack/wraith-browser-${V_BARE}-x86_64-macos
  mkdir -p /tmp/wb-pack/wraith-browser-${V_BARE}-x86_64-macos
  cp target/x86_64-apple-darwin/release/wraith-browser /tmp/wb-pack/wraith-browser-${V_BARE}-x86_64-macos/
  cp LICENSE README.md /tmp/wb-pack/wraith-browser-${V_BARE}-x86_64-macos/ 2>/dev/null || true
  ( cd /tmp/wb-pack && tar -czf ~/wraith-browser-${V_BARE}-x86_64-macos.tar.gz wraith-browser-${V_BARE}-x86_64-macos )
'

# Pull the resulting tarballs into dist/${VERSION}/:
scp imac:~/wraith-browser-${V_BARE}-aarch64-macos.tar.gz "${DIST}/"
scp imac:~/wraith-browser-${V_BARE}-x86_64-macos.tar.gz  "${DIST}/"

# Then re-run this script with --publish (it will regenerate SHA256SUMS.txt).
EOF
log "macOS: skipped on Kokonoe — see ${MAC_INSTRUCTIONS} for ssh imac commands"

# ---------------------------------------------------------------------
# 6. SHA256SUMS.txt
# ---------------------------------------------------------------------
log "generating SHA256SUMS.txt"
SUMS="${DIST}/SHA256SUMS.txt"
: > "$SUMS"
shopt -s nullglob
for f in "${DIST}"/wraith-browser-*.tar.gz "${DIST}"/wraith-browser-*.zip; do
  fname="$(basename "$f")"
  if command -v sha256sum >/dev/null; then
    h="$(sha256sum "$f" | awk '{print $1}')"
  elif command -v shasum >/dev/null; then
    h="$(shasum -a 256 "$f" | awk '{print $1}')"
  else
    h="$(certutil -hashfile "$f" SHA256 2>/dev/null | sed -n '2p' | tr -d ' \r')"
  fi
  echo "${h}  ${fname}" >> "$SUMS"
done
shopt -u nullglob
log "wrote ${SUMS}"

# ---------------------------------------------------------------------
# 7. Summary
# ---------------------------------------------------------------------
log "============================================"
log " build summary — ${VERSION}"
log "============================================"
( cd "$DIST" && ls -1 wraith-browser-* SHA256SUMS.txt 2>/dev/null | sed 's|^|  |' ) >&2
if [[ ${#FAILED_TARGETS[@]} -gt 0 ]]; then
  log "FAILED targets: ${FAILED_TARGETS[*]}"
  log "logs in ${LOG_DIR}/"
fi

# ---------------------------------------------------------------------
# 8. Optional: gh release create
# ---------------------------------------------------------------------
if [[ "$PUBLISH" -eq 1 ]]; then
  if [[ ${#FAILED_TARGETS[@]} -gt 0 ]]; then
    fail "refusing to --publish with failed targets: ${FAILED_TARGETS[*]}"
  fi
  log "publishing ${VERSION} to suhteevah/wraith-browser via gh release create"
  gh release create "${VERSION}" \
    --repo suhteevah/wraith-browser \
    --title "wraith-browser ${VERSION}" \
    --notes-file "${REPO_ROOT}/CHANGELOG_FRAGMENT.md" \
    "${DIST}"/wraith-browser-* \
    "${SUMS}"
  log "release published"
fi

if [[ ${#FAILED_TARGETS[@]} -gt 0 ]]; then
  exit 1
fi
log "done."
