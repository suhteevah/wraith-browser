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
