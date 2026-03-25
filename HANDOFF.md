# Wraith Browser — Session Handoff (2026-03-24, updated end-of-session)

## FIRST THING NEXT SESSION

**Rename the local directory:**
```bash
# From a different terminal/location (not inside the directory):
mv "J:/openclaw-browser" "J:/wraith-browser"
```
Then start Claude from `J:/wraith-browser`.

## Session Summary

Massive session: recovered from crashed deployment, reconciled git branches, deployed docs site, processed two external reviews, completed full OpenClaw → Wraith rebrand across 53 files, created v0.1.0 release, added benchmarks, expanded all thin content, and fixed SEO.

## What Was Done

### 1. Git Branch Reconciliation
- Local `main` had 62 commits, remote `origin` (openclaw-browser, archived) had 15 — no common ancestor
- Quarantined 7 remote-only files to `.quarantine/remote-only/`
- Pushed clean history to `wraith` remote (suhteevah/wraith-browser)
- Fixed 130MB `node_modules/next-swc` binary in git history via soft reset

### 2. Docs Site Deployed
- **Live URL**: https://wraith-browser.vercel.app
- Vercel project: `wraith-docs`, manually aliased to `wraith-browser.vercel.app`
- Disabled Vercel Authentication (was blocking public access)
- Dynamic OG image, sitemap.xml, robots.txt, custom 404, Twitter cards

### 3. Two External Reviews Processed
**Review 1 (wraith-browser-fixes.pdf):** 13 issues, 10 fixed
- Tool count aligned to 130 everywhere
- "Servo-derived" → "html5ever-based parsing"
- README trimmed 1233 → 165 lines
- Clone URLs, MCP tool names, enterprise link, PayPal badge all fixed

**Review 2 (wraith-docs-review.pdf):** 13 issues, 11 fixed
- Getting Started pages expanded (350→5.8K, 700→7.2K, 800→9.5K chars)
- MCP tool reference pages expanded (2.2K→9.5K, 2.4K→14.3K, 2.5K→11.2K)
- Blog dates staggered (Mar 10, 14, 18, 21, 23)
- OG image fixed, architecture linked from landing page, playground CTA added
- Discord dead link → GitHub Discussions, Matrix "coming soon" removed

### 4. Complete OpenClaw → Wraith Rebrand
- 53 source files renamed (Rust crates, Cargo.toml, SQL, Docker, docs, site)
- Zero "openclaw" references remain in source (verified via grep)
- GitHub repo description updated

### 5. Bypass Language Reframed
- Legal concern raised by reviewer
- "bypass" → "handling", "stealth" → "compatibility", "CAPTCHA solving" → "CAPTCHA integration"
- Features documented but not marketed as circumvention

### 6. v0.1.0 Release Published
- Tag pushed and release created via GitHub browser UI
- Release notes with highlights, install instructions, Claude Code connect command
- No prebuilt binaries yet (noted in release)

### 7. Benchmarks Added
- `benchmarks/` directory with 4 scripts: latency, memory, concurrency, token savings
- Token savings benchmark: raw HTML vs Wraith snapshot compression (95%+ savings pitch)
- README in benchmarks/ with methodology and business case

### 8. Community & Showcases
- 4 showcase entries: LLM Token Savings, Research Assistant, Docs Search Index, Price Monitor
- GitHub kanban seeded with 9 issues across 4 priority tiers

### 9. SEO Fixes
- metadataBase, sitemap, robots all pointing to wraith-browser.vercel.app (was wrong domain)
- Google/Bing sitemap ping endpoints deprecated — needs manual Search Console verification

## Current State

| Component | Status | URL/Location |
|-----------|--------|-------------|
| Docs site | Live, public, SEO-ready | https://wraith-browser.vercel.app |
| GitHub repo | Active, v0.1.0 released | https://github.com/suhteevah/wraith-browser |
| GitHub kanban | 9 issues seeded | Issues tab on repo |
| Rebrand | Complete | Zero openclaw refs |
| Search indexing | NOT INDEXED | Needs GSC + Bing Webmaster |
| Local directory | Needs rename | J:/openclaw-browser → J:/wraith-browser |
| Auto-deploy | Not set up | Manual vercel + alias each time |

## Remaining TODO

1. **Rename local directory** — `J:/openclaw-browser` → `J:/wraith-browser` (first thing next session)
2. **Google Search Console** — verify site, submit sitemap (manual, browser-based)
3. **Bing Webmaster Tools** — same
4. **Connect Vercel to GitHub** — eliminates manual deploy + alias dance (issue #6)
5. **Rotate the GitHub PAT** — it was exposed in conversation history

## Infrastructure Notes

- **Proxy for production scraping**: Use commercial residential proxies (BrightData/Oxylabs/IPRoyal) via Wraith's `--proxy` flag. No FOSS residential option exists. Tor is built-in but widely blocked.
- **Deploy process** (until auto-deploy): `cd foss-site && vercel --prod --yes && vercel alias <url> wraith-browser.vercel.app`
- **Vercel auth token**: `C:/Users/Matt/AppData/Roaming/com.vercel.cli/Data/auth.json`
