# Wraith Browser — Next Up

> Written: 2026-04-12
> Source: Cross-project analysis from "optimize my week" session

---

## BLOCKING: TRW Depends on Wraith

The Right Wire now has a two-pool scraper that routes stale X sources through Wraith's enterprise API (`/navigate` endpoint on WRAITH_API_URL). **If Wraith isn't running and reachable, 35 X sources will continue returning zero content.**

The TRW cron job calls:
```
POST ${WRAITH_API_URL}/navigate
{
  url: "https://x.com/{handle}",
  wait_for: "article[data-testid='tweet']",
  timeout: 15000,
  extract: "html"
}
```

And also uses:
```
POST ${WRAITH_API_URL}/auth/login
POST ${WRAITH_API_URL}/swarm/fan-out
```

**Action needed:** Verify Wraith's API server is running and these endpoints work. If not, get it deployed.

---

## Bug Reports

### BR-1: Enterprise API server status unknown ✅ RESOLVED 2026-05-01
The API server is now live at `http://207.244.232.227:8080` (pixie VPS, co-tenant with TRW). Compose stack at `deploy/corpo/`. Set `WRAITH_API_URL=http://207.244.232.227:8080` in TRW's Vercel project. Domain TBD (free hostname first, `.com` later) → Caddy in front for TLS once DNS resolves.

### BR-2: Pre-built binaries don't exist ✅ FIXED 2026-05-02
No GitHub Actions (banned). Need a local cross-compilation script. This blocks anyone else from using Wraith. Low priority unless shipping to customers.
**Implemented:** `scripts/build-release.sh` (+ PowerShell wrapper `scripts/build-release.ps1`) — Docker-based cross-compile for linux x86_64/aarch64, native MSVC for windows, parallel-builds with per-target log files, sha256sums, optional `--publish` to GitHub Releases via `gh`. macOS targets (x86_64 / aarch64) require a separate ssh imac run — the script writes the exact heredoc to `dist/$VERSION/_macos-build.txt`. First build ~15 min wall (aarch64-linux dominates via QEMU); subsequent builds ~30-60s per target with caches warm. Static-linking decision: glibc 2.36+ (not musl, not crt-static) — documented in `scripts/release-targets.md` because rquest/boring-sys, ort dlopen, and arti-client+wasmtime+tantivy combo are too fragile under musl right now.

### BR-3: Exposed GitHub PAT — runbook ready
PAT was exposed in a conversation transcript (noted in HANDOFF.md TODO #4). Needs rotation.
**Audit done 2026-05-02:** repo + git history are clean (no `ghp_` / `github_pat_` strings). The exposed token lives in Matt's `GH_TOKEN` env var on Kokonoe (prefix `github_pat_11AAR32JY...`). Rotation is purely a github.com + `setx` task — runbook at `scripts/rotate-github-pat.md` with the fine-grained-PAT scope recommendations (Contents R/W, Metadata R, Pull requests R/W, Workflows R; everything else off). 5 minutes to execute, can't be done from this side.

### BR-4: Migration error swallowed at boot ✅ FIXED 2026-05-01
`crates/api-server/src/main.rs` ran `sqlx::migrate!` and on failure logged `tracing::warn!` from the `wraith_enterprise` target, but the default `RUST_LOG=wraith_api_server=info,...` dropped that target on the floor. The banner just said `"Migrations: skipped (error)"` with no detail, and every endpoint then 500'd with `{"error":"database_error"}`. **Fix:** migration failure is now `eprintln!` + `process::exit(1)` — fail loud instead of booting a useless server. (Hit during initial pixie deploy when the default `postgres:16-alpine` image had no `vector`/`pgcrypto`/`uuid-ossp` extensions; compose now uses `pgvector/pgvector:pg16`.)

### BR-5: `POST /auth/register` 500s without `display_name` ✅ FIXED 2026-05-01
`RegisterRequest.display_name: Option<String>` but `users.display_name` column is `NOT NULL`. Sending a request without the field bound a NULL and the insert failed with `null value in column "display_name" of relation "users" violates not-null constraint`. **Fix:** the handler now defaults `display_name` to the email local-part when omitted or empty (`crates/api-server/src/routes/auth.rs`).

---

## Feature Requests

### FR-1: Wire HttpTransport into sevro-headless (HANDOFF TODO #5) ✅ FIXED 2026-05-02
Replace direct reqwest calls in sevro-headless with the `HttpTransport` trait. This enables the no_std path for ClaudioOS bare-metal. Trait exists in `crates/transport/`, just not wired in yet.
**Implemented:** the `transport: Arc<dyn HttpTransport>` field is now ungated and available in both std and no-std builds (`Option<Arc<dyn HttpTransport>>`, populated by std `new()`, left `None` by no-std `new()` until the caller swaps it in). Added `SevroEngine::with_transport(config, transport)` constructor available in both modes — that's the bare-metal entry point. Added `SevroEngine::fetch(url) -> Result<String, String>` helper that uses the trait directly (no reqwest, no QuickJS, no DOM parse) — works in both modes. Std `cargo check -p sevro-headless` and `cargo check -p sevro-headless --no-default-features` both pass. The deeper refactor of the 20+ remaining direct `self.client` reqwest calls inside the std-gated methods is left for a follow-up — that's a quality-of-life cleanup, not a blocker for the bare-metal path which now has a clean entry point via `with_transport().fetch()`.

### FR-2: `wraith run` CLI subcommand (HANDOFF TODO #7) ✅ FIXED 2026-05-01
Load YAML playbooks from `playbooks/`, validate variables, dispatch steps. Playbook parser already implemented, MCP tool exists — just needs a CLI entrypoint. Real use case: `wraith run sofascore-tennis`.
**Implemented:** new `Run` clap subcommand in `crates/cli/src/main.rs`. Resolves bare names (`sofascore-tennis`) against `./playbooks` (or `--playbook-dir`/`~/.wraith/playbooks`), accepts explicit paths, validates `--var key=value` overrides against the playbook's `variables` block, spins up the engine via the same `create_engine_with_options` helper as `Navigate`/`Task`, then walks `PlaybookStep` entries dispatching navigate / navigate_cdp / wait / eval_js / screenshot / verify directly against the `BrowserEngine` trait. Other actions (click, fill, upload, conditional, repeat, etc.) return `status: "skipped"` with a pointer to the MCP server — full coverage stays on `browse_run_playbook`. Output formats: `json` (default), `snapshot`, `markdown`, `raw` (emits the first `store_as` runtime value, useful for shell pipes). Exits non-zero on any step failure.

### FR-3: HTTP-only stealth mode (HANDOFF TODO #8) ✅ FIXED 2026-05-02
`wraith fetch <url>` with TLS fingerprinting for JSON APIs (no DOM needed). Use case: Sofascore, ESPN. Could be a library function: `use wraith_browser_core::stealth_fetch`. Fast path that skips Servo entirely.
**Implemented:** library re-export `wraith_browser_core::{stealth_fetch, has_stealth_tls}` at the crate root (the implementation already existed in `crates/browser-core/src/stealth_http.rs`). New `Commands::Fetch` clap subcommand in `crates/cli/src/main.rs` with `--user-agent`, `--accept-language`, and `--output body|headers|json` flags. Smoke-tested against `https://example.com` — status 200 round-trip in ~200ms. Without `--features stealth-tls` the binary uses standard reqwest (rustls TLS fingerprint, will be flagged by Cloudflare); rebuild with `cargo build --release --features stealth-tls` for the Firefox 136 BoringSSL emulation path.

### FR-4: Bare-metal integration testing (HANDOFF TODO #6) — coordination doc ready, blocked on ClaudioOS session
Compile-verify `wraith-dom`, `wraith-transport`, `wraith-render` in ClaudioOS repo. Wire into kernel. These crates are in `J:\baremetal claude\crates\`.
**Status 2026-05-02:** all 3 crates compile clean in the ClaudioOS workspace (only unused-helper warnings). **Real gap surfaced:** the ClaudioOS-side `wraith-transport` crate is a homonym, not a dependent — it defines its own `SmoltcpTransport` struct but never `impl HttpTransport for SmoltcpTransport`. The doc comment at the top promises the bridge but the code doesn't deliver. Cross-repo coordination doc written at `J:\baremetal claude\docs\WRAITH-CRATES-HANDOFF.md` — covers (a) the cross-repo dependency choice (recommend Option B: vendor the trait into ClaudioOS for now, escalate to publishing later), (b) the 4-line `impl HttpTransport for SmoltcpTransport` body the next ClaudioOS session needs to write, (c) the kernel-side `with_transport(...).fetch(url)` smoke test to verify end-to-end. Wraith-side is fully ready (FR-1 just landed `with_transport` + `fetch` helpers in both std and no-std builds). Marking this remains-pending on the wraith side until the ClaudioOS session reports the QEMU smoke test green.

---

## Priority Order

1. **BR-1** — Get API server running (TRW blocked)
2. **FR-1** — HttpTransport wiring (ClaudioOS path)
3. **FR-3** — Stealth fetch mode (high value, relatively small)
4. **FR-2** — CLI playbook runner (quality of life)
5. **FR-4** — Bare-metal integration (ClaudioOS dependency)
6. **BR-2** — Pre-built binaries (nice to have)
7. **BR-3** — PAT rotation (security hygiene)
