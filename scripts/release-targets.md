# wraith-browser — Release Targets

> Scope: the open-source `wraith-browser` binary (`crates/cli/`).
> NOT the proprietary `wraith-enterprise` binary (`crates/api-server/`) — that has
> its own deploy pipeline at `deploy/corpo/`.

## Targets shipped per release

| Triple                       | Builder                  | Where        | Archive                                                  |
|------------------------------|--------------------------|--------------|----------------------------------------------------------|
| `x86_64-unknown-linux-gnu`   | Docker `rust:1.88-slim`  | Kokonoe      | `wraith-browser-${V}-x86_64-linux.tar.gz`                |
| `aarch64-unknown-linux-gnu`  | Docker buildx arm64      | Kokonoe      | `wraith-browser-${V}-aarch64-linux.tar.gz`               |
| `x86_64-pc-windows-msvc`     | native `cargo --release` | Kokonoe      | `wraith-browser-${V}-x86_64-windows.zip`                 |
| `x86_64-apple-darwin`        | `cargo --release` cross  | imac (SSH)   | `wraith-browser-${V}-x86_64-macos.tar.gz`                |
| `aarch64-apple-darwin`       | `cargo --release` native | imac (SSH)   | `wraith-browser-${V}-aarch64-macos.tar.gz`               |

Kokonoe builds linux x86_64 + linux aarch64 + windows in parallel.
macOS targets are skipped on Kokonoe — the script emits the exact `ssh imac …`
command to run for each. Drop the resulting tarballs into `dist/${V}/` before
`--publish`.

## Why these targets

- **linux gnu (x86_64 + aarch64)** — covers cloud VPSes (Pixie/Vultr/Hetzner
  x86_64) and graviton / Pi / Apple-silicon-via-Docker (aarch64). Most users
  will pull these.
- **windows-msvc** — Matt builds and dogfoods on Kokonoe; this is the dev-loop
  binary.
- **macOS x86_64 + aarch64** — separate archives, not a universal binary. Lets
  M-series users grab the small one.

## Why no musl

The dep tree fights musl hard:

- `rquest` ships BoringSSL via `boring-sys`; tested clean against glibc, not
  musl. Custom TLS fingerprint setup falls over inside Alpine cross-images.
- `ort = "2.0.0-rc.12"` is `features = ["load-dynamic"]` — it `dlopen`s
  `libonnxruntime.so` at runtime. A fully-static musl binary defeats the
  purpose; ort upstream only ships glibc dylibs.
- `rusqlite` is `bundled`, fine on musl in isolation, but with `arti-client +
  tantivy + wasmtime` in the same link unit the musl path has historically
  wedged on `__pthread_register_cancel` / TLS fast-path bugs.
- `wasmtime 42` and `arti-client 0.40` both prefer glibc; arti has a soft
  dep on `getaddrinfo` quirks that BSD-libc / musl handle differently.

**Decision: build against `rust:1.88-slim-bookworm` (Debian 12, glibc 2.36).**
The resulting binaries run on any distro with glibc ≥ 2.36 — Debian 12+,
Ubuntu 22.04+, RHEL 9+, Fedora 36+. We don't pass `+crt-static` (it doesn't
help much for gnu, and it breaks `ort` `load-dynamic`).

If a customer needs an Alpine binary, that's a one-off fork of the script with
`rust:1.88-alpine` + `--target *-musl` and a willingness to debug.

## Why rust 1.88+

`time-core 0.1.8`, `cookie_store 0.22.1`, `home 0.5.12`, `time 0.3.47`,
`time-macros 0.2.27` all require rustc ≥ 1.88. Hit during the corpo deploy
2026-05-01. The builder image is pinned to `rust:1.88-slim-bookworm`.

## Output layout

```
dist/${VERSION}/
  wraith-browser-${V}-x86_64-linux.tar.gz
  wraith-browser-${V}-aarch64-linux.tar.gz
  wraith-browser-${V}-x86_64-windows.zip
  wraith-browser-${V}-x86_64-macos.tar.gz       # from imac
  wraith-browser-${V}-aarch64-macos.tar.gz      # from imac
  SHA256SUMS.txt
```

## Cache layout

Docker builds bind-mount these to keep the cargo cache hot across runs:

- `~/.cargo/registry`         → `/usr/local/cargo/registry`
- `dist/.cache/target-x86_64-linux`  → `/build/target` (per-triple)
- `dist/.cache/target-aarch64-linux` → `/build/target`

Source is bind-mounted read-only at `/build` (covers `crates/` and
`sevro/ports/headless/` because the workspace lives in one root). No `COPY` —
the workspace is ~1 GB and copying every run is wasted I/O.

Native windows build uses the regular `target/release/`.

## Publish flow

`--publish` runs:

```
gh release create ${VERSION} \
  --repo suhteevah/wraith-browser \
  --title "wraith-browser ${VERSION}" \
  --notes-file CHANGELOG_FRAGMENT.md \
  dist/${VERSION}/*
```

If `CHANGELOG_FRAGMENT.md` is missing, `--publish` aborts. Generate it before
publishing — the script does NOT auto-write release notes.

## Re-running

The script wipes `dist/${VERSION}/` on start unless it's non-empty, in which
case it refuses without `--force`. The cargo / docker layer caches under
`dist/.cache/` survive — only the per-version archive directory is reset.
