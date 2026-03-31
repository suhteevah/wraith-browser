# Wraith Browser — Session Handoff (2026-03-30)

## Session Summary

Wired FingerprintConfig into SevroEngine (auto-generates per session, passes to DOM bridge every page load), fixed ROI calculator's awkward self-hosted cost math, confirmed FlareSolverr beats Indeed's Cloudflare CAPTCHA (status 200 + cf_clearance), fixed challenge detection to recognize Indeed's "Security Check" page pattern, and fixed a Chrome136 TLS mismatch in the cookie retry path.

## What Was Done This Session

### 1. FingerprintConfig Wired into SevroEngine (COMPLETE)

Previously: FingerprintConfig existed in browser-core and the DOM bridge accepted it, but SevroEngine never created or passed one.

**Now:**
- `SevroEngine` has a `fingerprint: Option<HashMap<String, serde_json::Value>>` field
- `set_fingerprint()` public setter accepts a HashMap of property overrides
- `load_page_with_scripts()` passes `self.fingerprint.as_ref()` to `setup_dom_bridge_with_fingerprint()` — every page load gets Camoufox-style DOM spoofing
- `SevroEngineBackend` (browser-core) auto-generates fingerprint via `FingerprintConfig::generate().to_map()` in all constructors (`new()`, `with_config()`, `new_with_options()`)
- Confirmed in logs: `Generated fingerprint config screen=1536x864 cores=16 memory=4 dpr=1.25 canvas_seed=1006282108`

**Files changed:**
- `sevro/ports/headless/src/lib.rs` — field, setter, wiring in load_page_with_scripts
- `crates/browser-core/src/engine_sevro.rs` — auto-generate in constructors

### 2. ROI Calculator Fixed (COMPLETE)

The self-hosted competitor costs were using a fake "$3.50/1K pages" rate that was back-calculated from VM assumptions. Users saw per-page pricing for something that's not actually billed per-page.

**Now:**
- Self-hosted competitors (Playwright/Puppeteer) use a VM-based cost model: `ceil(pages / (sessions × pages_per_session × 30)) × $80/VM`
- Each solution has explicit params: `vmCost: 80, sessionsPerVm: 6, pagesPerSession: 200`
- Dropdown shows `(~$80/VM)` for self-hosted options, `(~$X/1K pages)` for hosted
- Comparison table uses the same VM-based formula
- Footnote explains assumptions clearly

**File changed:** `foss-site/components/roi-calculator.tsx`

### 3. Indeed + FlareSolverr Testing

**FlareSolverr confirmed working against Indeed:**
- Direct curl to FlareSolverr `POST /v1` returned status 200, `cf_clearance` cookie, full rendered page
- `"Challenge not detected!"` — FlareSolverr's Chrome solved Cloudflare Turnstile transparently
- FlareSolverr runs at `http://localhost:8191`

**Wraith binary NOT yet working with Indeed:**
- Challenge detection (`is_cloudflare_challenge()`) was NOT recognizing Indeed's page — Indeed uses `"Security Check"` title and `INDEED_CLOUDFLARE_STATIC_PAGE` JS var, not standard CF signatures
- Added `"Security Check"` and `"CLOUDFLARE_STATIC_PAGE"` to challenge detection
- Added debug logging for challenge/block detection
- **Still needs testing** — rebuilt binary, but Chrome zombie processes from FlareSolverr needed cleanup first

### 4. Firefox136 TLS Consistency Fix

The `http_fetch_with_cookies()` method (Tier 3.5 cookie replay) was using `Emulation::Chrome136` instead of `Emulation::Firefox136`. This meant if FlareSolverr solved a challenge and we tried to replay cookies, the TLS fingerprint would mismatch (Firefox UA + Chrome TLS = instant red flag).

**Fixed:** All `Emulation::Chrome136` replaced with `Emulation::Firefox136` in sevro-headless.

**File changed:** `sevro/ports/headless/src/lib.rs`

### 5. FlareSolverr Chrome Zombie Cleanup

FlareSolverr spawns headless Chrome instances to solve CAPTCHAs but doesn't always clean them up. During testing, 85+ chrome.exe processes accumulated. Killed them with `taskkill //F //IM chrome.exe`.

**Note for future:** FlareSolverr process management is a known issue. Consider adding a `maxBrowsers` config or switching to a FlareSolverr fork with better cleanup.

## Previous Session Work (2026-03-27)

### Firefox 136 TLS Emulation
- rquest uses `Emulation::Firefox136` for TLS fingerprint matching
- All UAs switched to Firefox 136
- Removed sec-ch-ua headers, aligned Accept/Priority headers

### FingerprintConfig System (Camoufox Port)
- `FingerprintConfig::generate()` — randomized consistent profiles
- `apply_canvas_noise()` — Camoufox seeded LCG algorithm
- 20+ DOM properties template-driven via QuickJS bridge
- 8 tests passing

### Enterprise Docs Site (6 Pages — ALL COMPLETE)
- Pricing, Features, Comparison, Security, Licensing, ROI Calculator
- 3 tiers: Growth $199, Scale $799, Enterprise custom
- AGPL enforcement language on pricing + licensing pages
- Live at https://wraith-browser.vercel.app

## Current State

| Component | Status |
|-----------|--------|
| Docs site | Live at https://wraith-browser.vercel.app |
| Enterprise pages | ALL 6 COMPLETE |
| FingerprintConfig | Wired end-to-end: generate → engine → DOM bridge |
| Firefox TLS | Consistent Firefox136 across all code paths |
| ROI Calculator | Fixed — VM-based self-hosted cost model |
| FlareSolverr | Confirmed working against Indeed (direct test) |
| Indeed via binary | Challenge detection fixed, needs retest |
| Release binary | Built at target/release/wraith-browser.exe |
| Pre-built binaries | NOT AVAILABLE — users must build from source or Docker |
| GitHub CI | BANNED — must build locally, no GitHub Actions |

## Remaining TODO

1. **Retest Indeed via wraith binary** — rebuild done, challenge detection updated, need to verify Tier 3 escalation works end-to-end: `WRAITH_FLARESOLVERR=http://localhost:8191 ./target/release/wraith-browser.exe navigate "https://www.indeed.com/jobs?q=&l=95926"`
2. **Pre-built binaries** — No download page on Vercel site. Need local cross-compilation script for Linux x86_64, macOS arm64/x86_64, Windows x86_64. Can't use GitHub CI (banned). Options: local `cross` tool, or add a `/downloads` page with build instructions.
3. **Connect Vercel to GitHub** — eliminates manual deploy
4. **Google Search Console / Bing Webmaster** — submit sitemap
5. **Rotate GitHub PAT** — was exposed in earlier conversation

## Key Technical Decisions

- **Firefox over Chrome for TLS** — Cloudflare targets Chrome fingerprints more aggressively. Firefox 136 via rquest passes basic Cloudflare. Does NOT pass Cloudflare Turnstile CAPTCHA without FlareSolverr.
- **Camoufox technique at Rust/QuickJS level** — No external binary dependency. Intercepts at DOM bridge, invisible to JS inspection.
- **FingerprintConfig auto-generated per engine** — Each engine instance gets a unique randomized fingerprint. Consistent within a session (same canvas seed, same screen size, etc.)
- **VM-based cost model for ROI** — Self-hosted competitors priced by VM count, not fake per-page rates. Honest comparison.
- **No GitHub CI** — Matt is banned. All builds must be local. Cross-compilation for release binaries TBD.
- **3-tier pricing** — Self-hosting from AGPL repo IS the free tier. Growth $199, Scale $799, Enterprise custom.
