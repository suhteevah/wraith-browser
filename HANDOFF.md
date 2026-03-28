# Wraith Browser — Session Handoff (2026-03-27)

## Session Summary

Fixed the broken Chrome TLS bypass by switching to Firefox 136 emulation (rquest + Camoufox technique), built the entire enterprise side of the docs site (6 pages), simplified pricing to 3 tiers, added AGPL enforcement language, and deployed everything.

## What Was Done

### 1. Firefox 136 TLS Emulation (Bypass Fix)

The original `stealth_fetch_rquest` was building a bare `rquest::Client` without calling `.emulation()` — sending a default BoringSSL fingerprint that Cloudflare flagged instantly. Additionally, all HTTP requests were sending Chrome `sec-ch-ua` headers alongside a Firefox User-Agent, which is an instant red flag.

**Fixed:**
- rquest now uses `Emulation::Firefox136` from `rquest-util` for TLS-level fingerprint matching
- All User-Agent strings switched from Chrome 131/136 to Firefox 136 across Sevro, Native, and stealth_http engines
- Removed all `sec-ch-ua` headers (Chromium-only) from sevro/ports/headless/src/lib.rs (3 locations)
- Default Accept-Language changed to `en-US,en;q=0.5` (Firefox style)
- Added `Priority: u=0, i` header (Firefox sends this, Chrome doesn't)
- Updated Accept header to Firefox format (no `image/apng`)
- Added `rquest-util = "2.2"` to workspace and browser-core deps

**Files changed:** Cargo.toml, crates/browser-core/Cargo.toml, crates/browser-core/src/stealth_http.rs, crates/browser-core/src/native.rs, sevro/ports/headless/src/lib.rs

### 2. FingerprintConfig System (Camoufox MaskConfig Port)

Studied Camoufox's open source patches (github.com/daijro/camoufox) — they intercept browser API property getters at the C++ level via a JSON config (`CAMOU_CONFIG` env var → `MaskConfig.hpp`). Ported this to Rust:

- `fingerprint_config.rs` — JSON-based config with dot-notation keys (`window.innerWidth`, `navigator.userAgent`, etc.)
- `FingerprintConfig::generate()` — produces randomized but consistent profiles (common screen resolutions weighted by real-world usage, realistic hardware specs, Firefox navigator values)
- `apply_canvas_noise()` — direct port of Camoufox's `CanvasFingerprintManager::ApplyCanvasNoise` (seeded LCG PRNG, modifies one RGB channel per pixel by ±1, skips zero channels to preserve transparency)
- 8 tests, all passing

**File:** crates/browser-core/src/fingerprint_config.rs

### 3. DOM Bridge Wiring

Updated the QuickJS DOM bridge (`dom_bridge.js`) to accept fingerprint config values instead of hardcoded Chrome defaults. 20+ properties are now template-driven:

- Window: innerWidth, innerHeight, outerWidth, outerHeight, screenX, screenY, devicePixelRatio
- Screen: width, height, availWidth, availHeight, colorDepth, pixelDepth
- Navigator: userAgent, language, languages, platform, hardwareConcurrency, maxTouchPoints, deviceMemory, oscpu, vendor (empty for Firefox), userAgentData (undefined for Firefox)

`js_runtime.rs` has a new `setup_dom_bridge_with_fingerprint()` method that accepts `Option<&HashMap<String, serde_json::Value>>`.

**Files changed:** sevro/ports/headless/src/dom_bridge.js, sevro/ports/headless/src/js_runtime.rs

### 4. TLS Profile Updates

- Added `firefox_136_profile()` as the new default
- Added `default_profile()` convenience function
- `firefox_132_profile()` now delegates to 136 with version-specific UA
- `all_profiles()` returns Firefox 136 first

**File:** crates/browser-core/src/tls_fingerprint.rs

### 5. Enterprise Docs Site (6 New Pages)

All at `foss-site/content/docs/enterprise/`:

| Page | File | Content |
|------|------|---------|
| Pricing | pricing.mdx | 3 tiers: Growth $199, Scale $799, Enterprise custom. Self-host is the free tier. |
| Features | features.mdx | RBAC, SSO/SAML, SOC 2, audit logging, data residency, credential mgmt, dedicated infra, priority support |
| Comparison | comparison.mdx | Feature matrix + cost comparison vs Browserbase, Browserless, Playwright, Puppeteer |
| Security | security.mdx | SOC 2, GDPR, encryption (AES-256-GCM, Argon2id), audit logging, vulnerability management |
| Licensing | licensing.mdx | AGPL-3.0 vs commercial license, use cases, FAQ, enforcement section |
| ROI Calculator | roi-calculator.mdx | Interactive React component — select current solution + volume, see cost comparison |

### 6. Site Navigation

- "Enterprise" added to top nav bar (layout.shared.tsx)
- "Scale with confidence" CTA section added to landing page (page.tsx)
- Enterprise section in docs sidebar (enterprise/meta.json)

### 7. AGPL Enforcement Language

Added to both pricing and licensing pages. Clear message: we monitor, we enforce, contribute back or pay.

### 8. Indeed Test Results

- **SimplyHired**: PASS at Tier 1 — 2,959 jobs found near 95926 with Firefox 136 emulation
- **Indeed**: Still 403 CAPTCHA — needs FlareSolverr (Tier 3) or Camoufox-level browser. Network was down during testing so FlareSolverr couldn't be tested.

## Current State

| Component | Status |
|-----------|--------|
| Docs site | Live at https://wraith-browser.vercel.app |
| GitHub | Pushed, commit aa7b464a |
| Firefox emulation | Working, compiles with `--features stealth-tls` |
| FingerprintConfig | Built, 8/8 tests pass, wired into DOM bridge |
| Enterprise pages | 6 pages live, linked from nav and landing page |
| Release binary | Built at target/release/wraith-browser.exe |
| Indeed bypass | Needs FlareSolverr or Camoufox integration for CAPTCHA |

## Remaining TODO

1. **Test Indeed with FlareSolverr** — `WRAITH_FLARESOLVERR=http://localhost:8191 ./target/release/wraith-browser.exe navigate "https://www.indeed.com/jobs?q=&l=95926"`
2. **Wire FingerprintConfig into SevroEngine** — the config exists and the DOM bridge accepts it, but `SevroEngine::navigate()` doesn't create/pass a config yet. Need to add a `fingerprint` field to `SevroConfig` or `SevroEngine` and pass it through to `setup_dom_bridge_with_fingerprint()`.
3. **Connect Vercel to GitHub** — eliminates manual deploy + alias dance (issue #6)
4. **Google Search Console / Bing Webmaster** — submit sitemap for indexing
5. **Rotate GitHub PAT** — exposed in earlier conversation history
6. **ROI calculator self-hosted tier** — currently shows $0/mo for self-hosted which is correct but the "savings" math is weird when comparing to self-hosted. May want to show estimated VM cost instead.

## Key Technical Decisions

- **Firefox over Chrome for TLS** — Cloudflare targets Chrome fingerprints more aggressively. Firefox 136 via rquest's `Emulation::Firefox136` passes Akamai, PerimeterX, and basic Cloudflare. Does NOT pass Cloudflare Turnstile CAPTCHA.
- **Camoufox technique, not Camoufox binary** — We studied their C++ patches and ported the approach to Rust. No external binary dependency.
- **3-tier pricing** — Free and Starter dropped. Self-hosting from the open source repo IS the free tier. Managed plans start at Growth ($199).
- **AGPL enforcement** — Explicitly stated on pricing and licensing pages. Legal action for violations.
