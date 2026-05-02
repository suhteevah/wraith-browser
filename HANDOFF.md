# Wraith Browser — Session Handoff

## Last Updated
2026-05-02

## Project Status
🟢 **Hosted Corpo API live** at `https://wraith-browser.vercel.app` (Vercel TLS-proxied) and `http://207.244.232.227:8080` (direct). Open AGPL repo + proprietary api-server tier both deploying clean. Growth-prep artifacts staged. FR queue cleared except for cross-repo FR-4 (waiting on ClaudioOS session).

## What's Next
1. **Pick a free hostname** (DuckDNS / FreeDNS / no-ip) → bring up Caddy via the staged `tls` profile in `deploy/corpo/docker-compose.yml` for real TLS + WSS. ~5 min once DNS resolves.
2. **Record the demo** following `marketing/recording-setup.md` on Kokonoe (~1.5-2h producer time). Ship the three outputs (`wraith-demo-{1080p,square,vertical}.mp4`) to `foss-site/public/` and redeploy.
3. **HN post** — final eyeball on `marketing/hn-launch.md`, schedule for Tue/Wed 8-9am PT.
4. **FR-4** — pick up `J:\baremetal claude\docs\WRAITH-CRATES-HANDOFF.md` in a ClaudioOS session: vendor the `HttpTransport` trait, write the 4-line `impl HttpTransport for SmoltcpTransport`, kernel smoke test in QEMU.
5. **BR-3** — Matt-action: rotate the exposed PAT per `scripts/rotate-github-pat.md` (5 min).

## Blocking Issues
- **Free hostname** for Caddy/TLS — none picked yet. Vercel rewrites cover REST but WebSocket upgrades and the cleartext Vercel→VPS leg still need real TLS on the VPS.
- **FR-4** on the ClaudioOS side — needs a separate session. Wraith side is fully ready (`SevroEngine::with_transport(...).fetch(url)` works in both std and no-std).

## Notes for Next Session
- `crates/api-server/` is **gitignored** (proprietary). Edits to it (BR-5 `display_name` fix, BR-4 migration-fail-loud) are local-only on Kokonoe + already deployed to pixie. Don't be surprised when `git diff` shows them as tracked-but-uncommitted; the gitignore was added after the file was first committed. Leave them out of public commits.
- The Anthropic API key in TRW's `.env.local` (`sk-ant-api03-l0…X_NgAA`) has **zero credit balance**. TRW production uses the OAuth-token proxy on pixie instead, so nothing's broken — but if you fall back to the key directly, top up first.
- `wraith-browser.vercel.app` is **aliased** to the `wraith-docs` Vercel project — not natively named. After every `vercel deploy --prod`, re-alias with `vercel alias set <new-url> wraith-browser.vercel.app` or the alias will revert.
- Edge rate-limit on `/api/v1/auth/register` is **per-region in-memory** (not durable). Sufficient for week-1 anti-abuse; swap for Vercel KV / Upstash before the HN post hits front page.
- The `target/` cache of the Docker linux builder lives at `dist/.cache/` — don't `rm -rf dist/` blindly, it'll add 15 min back to every release.

---

## Session Summary — 2026-05-02

This session: BR/FR sweep + growth-prep + hosted-API hardening + Anthropic model probe + FR-4 cross-repo handoff. See "Follow-on session 2026-05-02" entry below for the full per-task breakdown. The 2026-05-01 corpo deploy entry follows that.

## Session Summary — 2026-05-01

Deployed the proprietary corpo tier (`crates/api-server/`, `wraith-enterprise` binary, 77 REST endpoints + WS) to the Pixiedust VPS. Live at **`https://wraith-browser.vercel.app`** (Vercel TLS-proxied via `next.config.mjs` rewrites in `foss-site/`) and **`http://207.244.232.227:8080`** (direct, for WS + long calls). Co-tenant with the TRW pixiedust stack. Register + login JWT flow verified end-to-end on both URLs.

### What shipped

- `crates/api-server/Dockerfile` — multi-stage build, `rust:1.88-slim-bookworm` builder → debian-bookworm-slim runtime, non-root user `wraith` (uid 10001), HEALTHCHECK on `/health`. Build context = repo root (api-server has path-deps to `../browser-core` and `../../sevro/ports/headless`).
- `deploy/corpo/docker-compose.yml` — three-container stack on `wraith-corpo` network: `wraith-corpo-api` (`:8080`), `wraith-corpo-postgres` (`pgvector/pgvector:pg16`, internal), `wraith-corpo-redis` (`redis:7-alpine`, internal).
- `deploy/corpo/deploy.sh` — Kokonoe → pixie deploy: generates secrets on first run, streams the repo via `tar | ssh` (rsync isn't on Git Bash), runs `docker compose up -d --build`, polls `/health` for 5 min.
- `deploy/corpo/README.md`, `deploy/corpo/.env.example`.
- `.gitignore` updated for `deploy/corpo/.env*`.

### Build gotchas hit (and resolved)

1. `rust:1.83-slim` couldn't parse `time-core 0.1.8` (needs `edition2024`).
2. `rust:1.86-slim` rejected by `cookie_store@0.22.1`, `home@0.5.12`, `time@0.3.47`, `time-core@0.1.8`, `time-macros@0.2.27` (require rustc 1.88).
3. `rust:1.88-slim` worked — `Finished release profile [optimized] target(s) in 3m 17s`.
4. `postgres:16-alpine` had no `uuid-ossp` / `pgcrypto` / `vector` extensions → all migrations after `create_extensions` failed → no schema → every endpoint returned `{"error":"database_error"}`. **Switched to `pgvector/pgvector:pg16`.**

### Bugs found in api-server (NOT FIXED, just documented)

1. **Migration error swallowed at boot.** `main.rs` logs `tracing::warn!` from the `wraith_enterprise` binary target, but the default `RUST_LOG` filter only includes `wraith_api_server=info,...` — so migration failures don't appear in logs. Banner says `"Migrations: skipped (error)"` with no detail. Either widen the filter or `panic!` on migration failure.
2. **`POST /api/v1/auth/register` 500s without `display_name`.** `RegisterRequest.display_name: Option<String>` but `users.display_name` column is `NOT NULL`. Workaround: clients must send the field.

### Smoke results

```
GET  /health          → {"status":"ok","version":"0.1.0","db_connected":true}
POST /auth/register   → user + access_token + refresh_token (JWT HS256)
POST /auth/login      → access_token + refresh_token
```

### Remaining

- Pick a domain (free hostname first, `.com` later). Drop Caddy in front for TLS.
- Fix the two app bugs above. **DONE 2026-05-02** — `display_name` defaults to email local-part, migration error now fatal-loud. See `NEXT-UP.md` BR-4 / BR-5.
- Wire live Stripe keys when ready (currently `ENABLE_BILLING=false`).
- Move `VAULT_KMS_KEY_ID` off `local-dev-key` for production credential storage.

### Follow-on session 2026-05-02 — FR/BR sweep + growth prep + model probe + FR-4 prep

- **BR-3** GitHub PAT rotation runbook ready at `scripts/rotate-github-pat.md` (5 min Matt-action). Repo + git history clean of PAT remnants.
- **BR-2** Cross-compile release pipeline shipped: `scripts/build-release.sh` + `.ps1` wrapper, parallel Docker linux x86_64/aarch64 + native MSVC + ssh-imac for macOS. ~15 min wall on first build.
- **FR-1** HttpTransport now reachable from no_std builds — `transport: Option<Arc<dyn HttpTransport>>` ungated, `SevroEngine::with_transport(config, transport)` constructor + `fetch(url)` helper available in both modes. Both `cargo check` variants pass.
- **FR-2** `wraith run <playbook>` CLI subcommand wired (subagent did this).
- **FR-3** `wraith fetch <url>` CLI subcommand + `wraith_browser_core::stealth_fetch` re-export at the crate root. Smoke-tested against example.com.
- **FR-4** prep — coordination doc at `J:\baremetal claude\docs\WRAITH-CRATES-HANDOFF.md` for the next ClaudioOS session. Real blocker is the ClaudioOS-side `wraith-transport` crate being a homonym, not a dependent — it doesn't actually impl the trait. Doc spells out the 4-line bridge + the cross-repo dep choice (recommend vendoring for now).
- **Growth prep** — `/signup` page with anti-abuse layers (origin / honeypot / time-gate / edge-rate-limit) + optional Turnstile env-flag, `/vs` comparison page, beta pricing copy, `marketing/{hn-launch,demo-script,recording-setup}.md`. Site live at `https://wraith-browser.vercel.app`.
- **Anthropic model probe** — full `/v1/models` snapshot at `J:\llm-wiki\models\anthropic-api-snapshot-2026-05-02.md`. 9 models, no "mythos" reachable. Bloomberg article (2026-04-21) confirms it was a real model with a real auth bug; assumed patched.

### LLM wiki updates

- New: `J:\llm-wiki\projects\wraith-enterprise.md`
- Cross-refs added in `projects/wraith.md`, `projects/The Right Wire.md`, `fleet/Fleet Overview.md`, `index.md`
- Project memory: `C:\Users\Matt\.claude\projects\J--wraith-browser\memory\project_corpo_deploy.md`

---

# Wraith Browser — Session Handoff (2026-04-02)

## Session Summary

Massive session spanning 2026-03-31 to 2026-04-02. Fixed Indeed scraping (2 bugs), built ClaudioOS bare-metal port (3 new no_std crates, ~3,400 lines), created HttpTransport trait + feature-gated sevro-headless, added real head-to-head benchmarks vs Playwright, enterprise site polish (benchmarks page, contact page, downloads page with GitHub release), and added 3 new platform API hydrators for defense contractor career sites (Radancy, Phenom, Workday) — Boeing, L3Harris, Lockheed, MITRE, RTX all working via native engine.

## What Was Done This Session

### 1. Challenge Detection Size Guard Fix (COMPLETE)

`is_cloudflare_challenge()` had a `>50KB → return false` heuristic assuming large pages are real content. Indeed's CAPTCHA page is 61KB (bloated inline CSS), so it was never detected as a challenge.

**Fix:** Check definitive CF signatures (`CLOUDFLARE_STATIC_PAGE`, `cf-browser-verification`, `cf_chl_opt`) *before* the size guard. These are unambiguous — if present, it's a challenge page regardless of size. Weaker signatures still gated by the 50KB threshold.

**File changed:** `sevro/ports/headless/src/lib.rs` (lines ~1010-1035)

### 2. CLI `WRAITH_FLARESOLVERR` Env Var Fix (COMPLETE)

The `WRAITH_FLARESOLVERR` env var was only read by the MCP server (`crates/mcp-server/src/server.rs`). The CLI binary defined `--flaresolverr` as a clap flag but had no `env = "WRAITH_FLARESOLVERR"` attribute, so setting the env var did nothing.

**Fix:** Added `env = "WRAITH_FLARESOLVERR"` to the clap arg definition. Also added `"env"` to clap features in workspace `Cargo.toml`.

**Files changed:**
- `crates/cli/src/main.rs` — added `env = "WRAITH_FLARESOLVERR"` to clap arg
- `Cargo.toml` — added `"env"` to clap features

### 3. Indeed End-to-End Verification (CONFIRMED WORKING)

```
WRAITH_FLARESOLVERR=http://localhost:8191 ./target/release/wraith-browser.exe navigate "https://www.indeed.com/jobs?q=&l=95926"
```

Log trace:
1. `is_challenge=true` — detection now works on 61KB page
2. `Tier 2: QuickJS solver` — tried, failed (expected — can't solve Turnstile)
3. `Tier 3: Escalating to external solver` — env var now wired
4. `External solver returned page content html_len=1858442 status=200`
5. `Page: "Jobs, Employment in Chico, CA 95926 | Indeed"`

Chrome zombie count stayed stable (26→20 during test).

### 4. HttpTransport Trait (COMPLETE)

Created `crates/browser-core/src/transport.rs` — abstracts HTTP layer so different backends can be plugged in:
- `HttpTransport` trait with `async fn execute()` using RPITIT (Rust 1.75+, works in both std and no_std)
- `ReqwestTransport` impl wrapping `reqwest::Client` (current behavior)
- `TransportRequest`/`TransportResponse`/`TransportError` types using `BTreeMap` headers
- Ready to feature-gate behind `#[cfg(feature = "std")]` when the no_std split happens

**File added:** `crates/browser-core/src/transport.rs`
**File modified:** `crates/browser-core/src/lib.rs` (added `pub mod transport`)

### 5. Feature-Gated sevro-headless (COMPLETE)

Made `reqwest`, `tokio`, `rquickjs` optional behind a `std` feature flag:
- `default = ["std"]` — nothing breaks for normal builds
- 18 async methods gated with `#[cfg(feature = "std")]`
- `js_runtime` module entirely gated
- `#[cfg(not(feature = "std"))]` constructor variant for no_std builds
- `cargo check -p sevro-headless --no-default-features` compiles clean

**Files changed:**
- `sevro/ports/headless/Cargo.toml` — features + optional deps
- `sevro/ports/headless/src/lib.rs` — cfg gates on 18 methods, struct fields, imports
- `sevro/ports/headless/src/js_runtime.rs` — `#![cfg(feature = "std")]` at top

### 6. ClaudioOS Bare Metal Crates (COMPLETE — in ClaudioOS repo)

Built 3 new `#![no_std]` crates in `J:\baremetal claude\crates\`:

| Crate | Lines | Purpose |
|-------|-------|---------|
| `wraith-dom` | 1,610 | HTML parser + CSS selectors + form detection + text extraction |
| `wraith-transport` | 572 | HTTP/HTTPS over smoltcp TCP + embedded-tLS |
| `wraith-render` | 1,221 | DOM → styled character-cell grid for framebuffer panes |

**HTML parser research:** Evaluated html5ever, lol_html, tl, html5gum, quick-xml. html5ever/scraper are impossible to port (tendril + Servo selector chain deeply coupled to std). Built a custom zero-dependency parser instead — handles entity decoding, auto-close tags, CSS selectors, login form heuristic detection.

**Handoff doc for ClaudioOS session:** `J:\baremetal claude\docs\WRAITH-CRATES-HANDOFF.md`

### 7. Platform API Hydrators for Defense Contractor Sites (COMPLETE)

Added 3 new hydrators to sevro-headless that call career platform APIs directly, bypassing JS rendering entirely:

| Hydrator | Platform | Sites | API |
|----------|----------|-------|-----|
| `try_radancy_api_hydration` | Radancy/TalentBrew | Boeing, L3Harris, Lockheed | GET with `X-Requested-With: XMLHttpRequest` |
| `try_phenom_api_hydration` | Phenom People | MITRE, RTX | POST JSON to `/widgets` |
| `try_workday_api_hydration` | Workday | Honeywell | POST JSON, needs PLAY_SESSION cookie |

**Detection fix:** Platform hydrators now trigger regardless of element count. Previously nav chrome (>5 elements) caused the engine to skip hydration even on SPA shell pages.

**Verified results:**
- Boeing: 198 elements, no FlareSolverr needed
- L3Harris: 145 elements, no FlareSolverr needed
- Lockheed: 194 elements, no FlareSolverr needed
- MITRE: 180 elements, no FlareSolverr needed
- RTX: 246 elements, needs FlareSolverr (Cloudflare)

### 8. Enterprise Site Polish (COMPLETE)

- **Benchmarks page** — real head-to-head Wraith vs Playwright data (latency, memory, token savings)
- **Contact page** — "Talk to Sales" with GitHub Discussions as primary channel
- **Downloads page** — Windows binary on GitHub Releases v0.1.0
- **Gmail purged** — `ridgecellrepair@gmail.com` replaced across all enterprise pages
- **4 deploys** to wraith-browser.vercel.app

### 9. Indeed Login Flow Documentation (COMPLETE)

Documented Google SSO and email+password login flows via CDP tools at `scripts/indeed-login.md`. Enables page 2+ pagination once logged in. Cookies last ~7 days.

## Previous Session Work (2026-03-30)

- FingerprintConfig wired into SevroEngine (auto-generates per session)
- ROI Calculator fixed (VM-based self-hosted cost model)
- FlareSolverr confirmed working against Indeed (direct curl)
- Challenge detection signatures added for Indeed
- Firefox136 TLS consistency fix (cookie retry path)

## Previous Session Work (2026-03-27)

- Firefox 136 TLS emulation via rquest
- FingerprintConfig system (Camoufox port, 20+ DOM properties)
- Enterprise docs site (6 pages, all complete, live at wraith-browser.vercel.app)

## Current State

| Component | Status |
|-----------|--------|
| Docs site | Live at https://wraith-browser.vercel.app |
| Enterprise pages | ALL 6 COMPLETE |
| FingerprintConfig | Wired end-to-end: generate → engine → DOM bridge |
| Firefox TLS | Consistent Firefox136 across all code paths |
| ROI Calculator | Fixed — VM-based self-hosted cost model |
| FlareSolverr | Working end-to-end via binary |
| Indeed via binary | **WORKING** — Tier 3 FlareSolverr escalation confirmed |
| HttpTransport trait | DONE — `crates/browser-core/src/transport.rs` |
| sevro-headless no_std | DONE — feature-gated, `--no-default-features` compiles |
| Bare metal crates | DONE — wraith-dom, wraith-transport, wraith-render in ClaudioOS repo |
| Release binary | Built at target/release/wraith-browser.exe |
| Pre-built binaries | NOT AVAILABLE — users must build from source or Docker |
| GitHub CI | BANNED — must build locally, no GitHub Actions |

## Remaining TODO

1. **Pre-built binaries** — No download page on Vercel site. Need local cross-compilation script for Linux x86_64, macOS arm64/x86_64, Windows x86_64. Can't use GitHub CI (banned). Options: local `cross` tool, or add a `/downloads` page with build instructions.
2. **Connect Vercel to GitHub** — eliminates manual deploy
3. **Google Search Console / Bing Webmaster** — submit sitemap
4. **Rotate GitHub PAT** — was exposed in earlier conversation
5. **Wire HttpTransport into sevro-headless** — Replace direct reqwest calls with `HttpTransport` trait usage so the no_std path has a pluggable backend
6. **Bare metal integration testing** — ClaudioOS session needs to compile-verify the 3 new crates and wire them into the kernel. Handoff doc at `J:\baremetal claude\docs\WRAITH-CRATES-HANDOFF.md`
7. **FR: `wraith run` CLI subcommand for playbooks** — Playbook parser (`browser-core/src/playbook.rs`) and runner exist, but there's no CLI subcommand to execute them. Need `wraith run <playbook-name> --var key=value` that loads YAML from `playbooks/`, validates variables, and dispatches steps to the engine. The MCP server has `browse_run_playbook` but the CLI binary has no equivalent. Real-world use case: `wraith run sofascore-tennis` for stealth API scraping with CamoFox fingerprinting — currently impossible from CLI despite playbook YAML being ready at `playbooks/sofascore-tennis.yml`. Without this, users must fall back to Chrome headless subprocess calls which lack wraith's fingerprint rotation and stealth stack.
8. **FR: HTTP-only stealth mode (no DOM)** — For JSON API scraping (e.g., Sofascore, ESPN), a full browser engine is overkill. Need a lightweight `wraith fetch <url>` that applies CamoFox TLS fingerprinting + stealth headers to a plain HTTP request without spinning up QuickJS/DOM. This would let Rust projects use wraith as a stealth HTTP client library (`use wraith_browser_core::stealth_fetch`) rather than shelling out to Chrome headless. Key requirement: TLS fingerprint must match Firefox/Chrome (not Rust's default rustls JA3).

## Key Technical Decisions

- **Firefox over Chrome for TLS** — Cloudflare targets Chrome fingerprints more aggressively. Firefox 136 via rquest passes basic Cloudflare. Does NOT pass Cloudflare Turnstile CAPTCHA without FlareSolverr.
- **Camoufox technique at Rust/QuickJS level** — No external binary dependency. Intercepts at DOM bridge, invisible to JS inspection.
- **FingerprintConfig auto-generated per engine** — Each engine instance gets a unique randomized fingerprint. Consistent within a session (same canvas seed, same screen size, etc.)
- **Definitive CF signatures bypass size guard** — `CLOUDFLARE_STATIC_PAGE`, `cf-browser-verification`, `cf_chl_opt` checked before the 50KB heuristic. Prevents false negatives on bloated CAPTCHA pages.
- **VM-based cost model for ROI** — Self-hosted competitors priced by VM count, not fake per-page rates. Honest comparison.
- **No GitHub CI** — Matt is banned. All builds must be local. Cross-compilation for release binaries TBD.
- **3-tier pricing** — Self-hosting from AGPL repo IS the free tier. Growth $199, Scale $799, Enterprise custom.
- **`tl` over html5ever for bare metal** — html5ever/scraper are impossible to port (tendril + Servo deps deeply coupled to std). Built a custom zero-dep parser that handles real-world login pages.
- **Parallel Claude sessions** — Wraith session (this) handles wraith-browser repo. ClaudioOS session handles the bare-metal OS. Cross-session coordination via handoff docs in each repo's `docs/` directory.
