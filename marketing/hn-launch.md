# Show HN: Wraith Browser — a Rust browser engine built for AI agents, not humans

Wraith fetches a static page in **~50ms** and pulls full job listings off Boeing, L3Harris, Lockheed, and MITRE career sites in **145–200ms** — no Chrome, no Playwright, no headless anything. It's a native Rust browser engine I built because every "AI browser automation" stack today is a 300MB Chromium wrapper that takes 2–5 seconds just to start.

## Why it exists

LLMs don't need a viewport, a paint pipeline, or smooth scrolling. They need a DOM they can query, a JS runtime that runs the hydration scripts, and a fetch path that doesn't get instantly Cloudflare-banned. Everything else is overhead an agent pays for and never uses.

So Wraith is an html5ever DOM + a QuickJS runtime (rquickjs) + an rquest HTTP client doing Firefox 136 TLS emulation, exposed as **130 MCP tools** an agent calls directly. ~15MB binary. 50–100 concurrent sessions on 16GB instead of 6–8 with Playwright.

## What's actually working today

- **Native engine, no Chrome dependency.** `cargo build --release` gives you one binary. ~50ms per static page.
- **Defense-contractor talent platform hydrators** — the killer feature. Wraith detects Radancy, Phenom, and Workday backends and calls their internal JSON APIs directly instead of rendering the SPA. Verified end-to-end: Boeing 198 elements / 198ms, L3Harris 145ms, Lockheed 194ms, MITRE 180ms (no Cloudflare solver needed). RTX works too but routes through FlareSolverr.
- **3-tier challenge resolution** — Tier 1 direct fetch with Firefox TLS, Tier 2 in-process QuickJS challenge solver, Tier 3 FlareSolverr for Turnstile. Indeed scraping working end-to-end including the 61KB Cloudflare CAPTCHA pages that broke our size-guard heuristic last month.
- **130 MCP tools**: navigate, click, fill, extract markdown, eval JS, vault, knowledge graph, workflow record/replay, swarm parallel browsing, MCTS planning, time-travel debugging.
- **AES-256-GCM credential vault** with Argon2id KDF, per-domain approval, TOTP, Chrome cookie import.
- **Hosted Corpo tier** just deployed — 77 REST endpoints + WS, free during beta.

## What is NOT done yet

- **Bare-metal port** — the no_std crates (`wraith-dom`, `wraith-transport`, `wraith-render`, ~3,400 lines) compile and have tests, but aren't fully integrated into ClaudioOS yet.
- **Pre-built binaries** — build from source or Docker for now. (My GitHub CI is banned, no Actions builds.)
- **Vision ML** — ONNX inference is wired but the production models aren't downloaded by default.
- **WASM plugin runtime** — wasmtime hosting works, but the WIT interface isn't finalized.

## Try it

```bash
# Hosted (free during beta)
curl -X POST https://wraith-browser.vercel.app/api/v1/auth/register \
  -H 'Content-Type: application/json' \
  -d '{"email":"you@example.com","password":"...","display_name":"you"}'

# Direct VPS (use for WebSocket / long calls)
# http://207.244.232.227:8080

# Or self-host
git clone https://github.com/suhteevah/wraith-browser
cd wraith-browser && cargo build --release
claude mcp add wraith ./target/release/wraith-browser -- serve --transport stdio
```

## Pre-empting the obvious

- **"Why not Playwright/Puppeteer?"** They start Chrome. Chrome is 300MB, 2–5s startup, 300–500MB per session. For an agent doing 1000 page fetches an hour, that's the whole machine.
- **"Why not Browserbase?"** It's still Chrome behind an API, same per-page cost model, and your agent doesn't get a knowledge graph or MCP tools native.
- **"Why not just headless Chrome with stealth plugins?"** Because Cloudflare, Akamai, and PerimeterX are increasingly TLS-fingerprinting at the JA3/JA4 level. rquest with Firefox 136 emulation passes basic Cloudflare without any solver. Headless Chrome doesn't.
- **"Why AGPL?"** So companies that want to embed Wraith in proprietary SaaS pay for a commercial license. Self-hosting and personal use are free forever.

## What I'd love feedback on

1. The platform-API hydrator pattern. Is "detect the backend SPA framework and call its JSON API instead of rendering" generalizable past defense contractor career sites? What other site categories would benefit?
2. AGPL vs MPL vs Apache for an AI infrastructure project. Am I scaring off the wrong people?
3. The 130 MCP tools surface. Too much? Too little? Would you rather see 30 well-composed primitives?
4. What's missing from the "agent-first browser" vision that would make you actually rip Playwright out of your stack?

GitHub: https://github.com/suhteevah/wraith-browser
Docs: https://wraith-browser.vercel.app

---

## Title options

1. `Show HN: Wraith Browser – a Rust browser engine built for AI agents, not humans`
2. `Show HN: 50ms-per-page Rust browser engine with 130 MCP tools (no Chrome)`
3. `Show HN: Scraping Boeing/Lockheed/MITRE careers in <200ms by calling their SPA APIs directly`

Angle 1 is the safest — clear positioning, leans on the "for AI" framing HN has been chewing on all year. Angle 2 is the receipts-first version; quants in the title beat adjectives. Angle 3 is the most clickable but risks defense-contractor adversarial comments and "is this even legal" derail. **Recommend #2** — strongest signal-to-noise, hardest to dismiss.

## Posting checklist

**Best window:** Tuesday or Wednesday, **8:00–9:30am Pacific** (11am–12:30pm Eastern). Avoid Monday (clears weekend backlog), Friday (engagement falls off a cliff after lunch PT), and the entire US holiday week. The /show page has its own ranking but front-page lift still requires the early-morning PT slot when EU is winding down and US East is ramping up.

**Pre-flight (the night before):**
- README.md polished, repo pinned to top of GitHub profile
- A 30-second loom or asciicast of `wraith-browser navigate https://www.indeed.com/jobs?q=&l=95926` succeeding — embed link in the post or first comment
- The hosted endpoint smoke-tested from a clean network (Tailscale off, fresh IP)
- A pinned "FAQ" comment ready to paste as the first reply (covers AGPL, vs Servo, vs Playwright)

**Accounts to seed comments:**
- Drop a link in the Rust Discord `#showcase` and the rmcp / MCP Discord roughly 30 min after posting (not before — HN flags coordinated voting)
- Post to /r/rust as a parallel "I built X" thread linking back to the HN discussion, NOT the GitHub
- Ping any prior contacts who've expressed interest in browser automation — let them decide if they want to comment
- **Do NOT** ask for upvotes anywhere. HN flagging is brutal and irreversible.

**Expected first-hour comment patterns:**
1. "But Servo?" — answer: Servo is a rendering engine for humans; Wraith strips the renderer entirely and exposes the DOM as MCP tools. We use html5ever (Servo's parser) but not the rest of the stack.
2. "But Firefox / Gecko?" — answer: Same as Servo. Gecko is 20M+ LOC of viewport. Wraith is 27K LOC because an agent doesn't need to paint pixels.
3. "AGPL is anti-business" — answer: That's the point. Self-host free, embed-in-SaaS pays. Standard dual-license play.
4. "How is this different from Browser-Use / Stagehand?" — answer: Those are agent loops on top of Playwright. Wraith replaces Playwright. You can run a Browser-Use-style agent on top of Wraith and get the 50ms-per-page floor for free.
5. "Cloudflare is going to ban this in a week" — answer: Cloudflare bans Chrome JA3 fingerprints daily; Firefox is currently the lower-pressure target. Tier 3 FlareSolverr is the safety valve when Tier 1 stops working.
6. "Show me the benchmarks." — link to https://wraith-browser.vercel.app/docs/benchmarks (head-to-head Playwright data is already there).
7. "Why not write the corpo tier in Go / TS / Python?" — answer: The corpo tier IS Rust (axum + sqlx). Same language as the engine, single deploy artifact, 75MB statically linked.

**Comment hygiene:**
- Reply to every top-level comment in the first 2 hours. Engagement drives ranking.
- Never argue with downvoted commenters. Acknowledge, clarify, move on.
- If the post stalls below 5 points after 30 min, do NOT delete-and-repost. Let it ride.

## Backup channels

If HN underperforms (under 30 points by hour 4), fan out to these in priority order:

1. **Lobsters** — `practices` and `programming` tags. Post manually; Lobsters audience is more Rust-curious than HN.
2. **/r/rust** — "I built a Rust browser engine for AI agents" framing. Mod-friendly community; cite the html5ever / rquest / rquickjs dependencies up front.
3. **/r/programming** — broader audience, lower signal. Use the benchmarks angle, not the AI angle.
4. **This Week in Rust** — submit to the weekly newsletter PR queue. Free distribution to ~25K Rust devs.
5. **awesome-rust** — open a PR adding Wraith under "Web programming → Browser automation" once it's been live for a week and has stars.
6. **dev.to** — cross-post the launch text as an article. Tags: `rust`, `ai`, `webscraping`, `mcp`. Low effort, decent SEO long tail.
7. **Hacker News classifieds / "Who is hiring"** — irrelevant for launch but useful later when the corpo tier needs design partners.
8. **MCP Discord** + **Anthropic developer forum** — frame as "MCP server with 130 tools, here's what we learned" rather than a launch.
9. **Twitter/X** — quote-thread with the 50ms benchmark gif. Tag rust subreddit alumni who post browser-automation content.
10. **YC Startup School / Indie Hackers** — only if/when the corpo tier has paying customers; until then it's just noise.

If all of those underperform, the diagnosis is positioning, not channel. Rewrite the lede around a single concrete win story (the defense-contractor scrape, or a token-cost comparison vs Browserbase) and try Show HN again in 6–8 weeks under a different angle.
