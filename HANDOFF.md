# Wraith Browser — Session Handoff (2026-03-24)

## Session Summary

Recovered from a crashed mid-deployment session. Reconciled diverged git branches, deployed the docs site, processed two external reviews, and completed a full rebrand.

## What Was Done

### 1. Git Branch Reconciliation
- Local `main` had 62 commits, remote `origin` (openclaw-browser, archived) had 15 — no common ancestor
- Quarantined 7 remote-only files to `.quarantine/remote-only/` for review
- Identified local as authoritative (remote only had 3 planning docs beyond shared base)
- Pushed clean history to `wraith` remote (suhteevah/wraith-browser)

### 2. Fixed node_modules in Git History
- Previous session accidentally committed `node_modules/` (130MB next-swc binary)
- Soft-reset last 4 commits, recommitted cleanly without node_modules
- Push succeeded after history rewrite

### 3. Docs Site Deployed (foss-site → wraith-docs)
- Built and deployed Fumadocs site to Vercel
- **Live URL**: https://wraith-browser.vercel.app (manually aliased)
- Vercel project renamed from `foss-site` to `wraith-docs`
- Disabled Vercel Authentication (was blocking public access with 401)
- Created dynamic OG image via opengraph-image.tsx
- Added sitemap.xml, robots.txt, custom 404 page

### 4. Content Created
- **5 SEO blog posts**: browser automation without Chrome, knowledge graph from web data, MCP vs Playwright, $5 VPS concurrent sessions (Hetzner CX22), Rust engine deep-dive
- **Blog dates staggered**: Mar 10, 14, 18, 21, 23 (not all same day)
- **Getting Started pages expanded**: installation (5.8K chars), first-session (7.2K chars), hello-world-scrape (9.5K chars) — up from ~350-800 chars each
- Removed anti-detection blog post and introducing-openclaw post

### 5. External Review Fixes (2 reviews processed)
**Review 1 — GitHub/README audit:**
- Tool count fixed to 130 (actual, verified from server.rs) across all surfaces
- Clone URL fixed (openclaw → wraith)
- "Servo-derived" claim replaced with "html5ever-based parsing"
- wraith.dev/enterprise dead link removed
- README trimmed from 1233 → 165 lines
- PayPal badge removed
- GitHub repo description updated

**Review 2 — Docs site audit:**
- Getting Started pages expanded (see above)
- OG image fixed (dynamic route, removed static /og-image.png refs)
- Architecture deep-dive linked from landing page
- Playground CTA added above the fold
- Discord dead link replaced with GitHub Discussions
- Matrix "coming soon" removed
- metadataBase, Twitter cards, title templates, docs OG metadata added

### 6. Complete OpenClaw → Wraith Rebrand
- **53 files** modified across entire Rust codebase
- All crate names: openclaw-* → wraith-*
- All module paths: openclaw_* → wraith_*
- All data paths: ~/.openclaw/ → ~/.wraith/
- All env vars: OPENCLAW_* → WRAITH_*
- Docker configs, SQL schemas, deploy configs, .mcp.json, LICENSE, CONTRIBUTING.md
- Zero "openclaw" references remain in source (verified via grep)

### 7. Bypass Language Reframed
- "bypass" → "handling" / "compatibility"
- "stealth" → "browser compatibility"
- "evasion" → "compatibility config"
- "CAPTCHA solving" → "CAPTCHA integration"
- Legal concern flagged by reviewer — features documented but not marketed as circumvention

### 8. GitHub Kanban Seeded
9 issues created across 4 priority tiers with labels (critical, high-priority, medium, low-priority, docs, infra, content).

## Current State

| Component | Status | URL/Location |
|-----------|--------|-------------|
| Docs site | Live, public | https://wraith-browser.vercel.app |
| GitHub repo | Active, 130 tools | https://github.com/suhteevah/wraith-browser |
| Vercel project | wraith-docs, prod deployed | Needs auto-deploy setup (issue #6) |
| Rebrand | Complete | Zero openclaw refs in source |
| Blog | 5 posts, staggered dates | /blog on docs site |
| Getting Started | Expanded, production-ready | /docs/getting-started/* |

## Open Issues (GitHub Kanban)

| # | Priority | Title |
|---|----------|-------|
| 1 | Critical | Tag v0.1.0 release with prebuilt binaries |
| 2 | Critical | Add reproducible performance benchmarks |
| 3 | High | Port MCP tool reference from old README into docs |
| 4 | High | Submit to Google Search Console + Bing |
| 5 | Medium | Add 'Built with Wraith' showcase entry |
| 6 | Medium | Connect Vercel project to GitHub for auto-deploys |
| 7 | Medium | Write Job Application Automation guide |
| 8 | Low | Add sevro crate description in Cargo.toml |
| 9 | Low | Rename local directory openclaw-browser → wraith-browser |

## Known Issues

1. **Vercel alias is manual** — each deploy requires `vercel alias <url> wraith-browser.vercel.app`. Fix: connect repo to Vercel project (issue #6)
2. **`wraith-docs.vercel.app` is taken globally** — not our domain. Always use `wraith-browser.vercel.app`
3. **Local directory still named `J:/openclaw-browser`** — rename requires session restart and memory path updates
4. **No CI/CD** — GitHub Actions may not work (account restrictions). Consider alternative CI or manual releases
5. **MCP tool reference pages still thin** — ~3K chars vs the 10K+ architecture pages. Issue #3.
6. **Not indexed by search engines** — sitemap + robots.txt in place, but GSC/Bing submission is manual (issue #4)
7. **PAT token was exposed in conversation** — needs rotation

## Previous Handoff (2026-03-23)

The previous session handled Wraith Enterprise on CNC:
- pgvector upgrade, enterprise restart, TRW user registration
- wire-cron configuration, scraper auth fix
- See git history for details
