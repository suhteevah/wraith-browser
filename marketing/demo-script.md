# Wraith Browser — Launch Demo Recording

**Target length:** 75 seconds (sweet spot in the 60–90s window)
**Format:** Silent screen capture + on-screen captions (lower-thirds)
**Aspect ratios delivered:** 16:9 1080p (master), 1:1 1080×1080 (Twitter/LinkedIn feed), 9:16 1080×1920 (Reels/Shorts/TikTok)
**Soundtrack:** None. Captions only. (Rationale below in Production Notes.)

---

## Part 1 — Scene-by-Scene Script

### Arc rationale (read this first if you're tempted to "just record a demo")

The suggested arc is good but I'm tweaking the order so the moat lands earlier.
The single most defensible thing Wraith has is the **defense-contractor hydrators
returning structured JSON in <200ms while Playwright is still spinning up Chrome**.
That's the cold open. Everything after is amplifier — swarm fan-out proves it
scales, the MCP turn proves it's not a CLI demo, the silent CTA closes it.

We do NOT open with logos, taglines, or "Hi, I'm Matt." Open on raw terminal.
Anyone scrubbing through a muted Twitter video must understand the punch line
within 4 seconds or they're gone.

---

### SCENE 1 — Cold open: the latency gap (0:00 – 0:11)

**Duration:** 11s
**Layout:** Vertical split-screen, 50/50.
- **Left pane:** terminal labeled `Playwright`. Black background, red-tinted prompt.
- **Right pane:** terminal labeled `Wraith`. Black background, emerald-tinted prompt.
- Both terminals start with the prompt visible, no command yet.

**Action timeline:**

| t (s) | Left (Playwright) | Right (Wraith) |
|---|---|---|
| 0.0 | `$ time node playwright-boeing.js` is typed and ENTER pressed | `$ time curl -sX POST https://wraith-browser.vercel.app/api/v1/sessions/$SID/navigate \`<br>`  -H "Authorization: Bearer $TOK" -d '{"url":"https://jobs.boeing.com"}' \| jq` is typed and ENTER pressed |
| 0.5 | "Launching Chromium…" spinner appears | Status `200 OK` already prints. JSON pretty-prints (`{ "jobs": [ ... 198 entries ... ], "elapsed_ms": 198 }`). |
| 0.8 | Chromium spinner still going | `real 0m0.218s` prints. **Right pane goes still.** |
| 3.4 | Chromium loads page, jobs DOM finally ready, `real 0m3.412s` prints | (still) |
| 4.0 | Both panes idle. Numbers held side-by-side. | |

Hold the final frame from 4.0s → 11.0s. Seven seconds of dead air on the comparison is *deliberate* — it gives the viewer time to read both `real` numbers and do the math.

**Caption (lower-third, both panes spanned):**

> **Same job board. Same data. 17× faster.**
> Playwright: 3.412s · Wraith: 0.218s

Caption fades in at 4.5s, holds through end of scene.

**Why this scene first:** Most browser-automation videos open with "look at this cool thing it can do." That's already a 50-million-view genre on YouTube. The only thing that earns a click on HN/Twitter in 2026 is a **measurable, comparative claim in the first 5 seconds**. The split-screen makes the claim un-arguable. We're not showing capability — we're showing a **gap**.

---

### SCENE 2 — Inside the response (0:11 – 0:24)

**Duration:** 13s
**Layout:** Single full-screen terminal (the Wraith pane from Scene 1, smoothly zoomed in).

**Action timeline:**

| t (s) | Action |
|---|---|
| 11.0 | Crossfade from split → full-screen Wraith pane (0.4s ease) |
| 11.4 | Cursor appears, types `\| jq '.jobs[0:3]'` and ENTER |
| 12.0 | Three structured job objects render, indented JSON. Each shows `title`, `req_id`, `location`, `posted_at`, `apply_url`, `clearance_required`. |
| 14.0 | Cursor types `\| jq '.jobs \| length'` and ENTER |
| 14.6 | Output: `198` |
| 16.0 | Caption appears |
| 24.0 | End scene |

**Caption (lower-third, animated typewriter):**

> **Boeing's TalentBrew API. Hydrated, parsed, LLM-ready.**
> No JS rendering. No headless Chrome. No screenshot OCR.

**Why this scene here:** The cold open earned 11 seconds of attention with raw speed. Now we have to convert speed into *value*. The skeptic's first thought is "ok it was fast but did it actually get the data?" Showing 198 structured job objects with clearance flags answers that question and quietly seeds the defense-contractor positioning without any voiceover claiming it.

---

### SCENE 3 — Swarm fan-out across 4 contractors (0:24 – 0:38)

**Duration:** 14s
**Layout:** Single full-screen terminal, top half shows the command, bottom half shows a 4-row live progress display.

**Action timeline:**

| t (s) | Action |
|---|---|
| 24.0 | Terminal clears. Cursor types: `$ curl -sX POST https://wraith-browser.vercel.app/api/v1/swarm/fan-out \`<br>`  -H "Authorization: Bearer $TOK" \`<br>`  -d '{"urls":["https://jobs.boeing.com","https://careers.l3harris.com","https://lockheedmartinjobs.com","https://careers.mitre.org"]}'` |
| 26.5 | ENTER pressed |
| 26.7 | Four-row progress display appears, all rows show `[ … ] queued` |
| 27.0 | Row 1 (Boeing) flips to `[ ✔ ] 198 jobs · 184ms` |
| 27.1 | Row 3 (Lockheed) flips to `[ ✔ ] 194 jobs · 191ms` |
| 27.4 | Row 2 (L3Harris) flips to `[ ✔ ] 145 jobs · 213ms` |
| 27.6 | Row 4 (MITRE) flips to `[ ✔ ] 180 jobs · 167ms` |
| 28.0 | Bottom line prints: `swarm complete · 4/4 ok · 717 jobs · wall 1.83s` |
| 30.0 | Caption appears |
| 38.0 | End scene |

**Caption:**

> **One call. Four defense contractors. 717 structured jobs. 1.83 seconds.**
> Native Rust hydrators for Radancy, Phenom, Workday — no Chrome anywhere in the stack.

**Why this scene here:** Speed (Scene 1) + structure (Scene 2) is table stakes. Concurrency is the killer. A single Playwright worker would take 4 × 3.4s = 13.6s sequentially, or need a real browser pool to parallelize. Wraith does it in one HTTP call because the engine is so cheap a single VPS can fan out four sessions without flinching. This is the "couldn't do this with the old stack" moment.

---

### SCENE 4 — MCP: an LLM uses it mid-conversation (0:38 – 0:58)

**Duration:** 20s
**Layout:** Switch to a Claude Code terminal UI (or any MCP client — Claude Code is the most recognizable). Full-screen.

**Action timeline:**

| t (s) | Action |
|---|---|
| 38.0 | Crossfade to Claude Code interface. User prompt area focused. |
| 38.5 | User types: *"Pull the senior Rust openings from L3Harris and tell me which ones require a clearance."* |
| 41.0 | ENTER. Claude's reply streams in. |
| 41.5 | A tool-use block appears in-line: `mcp__wraith-browser__browse_navigate` with args `{ "url": "https://careers.l3harris.com" }` |
| 42.0 | Tool result block expands: snapshot returned with `145 elements` and a markdown excerpt of the page |
| 43.5 | Second tool call: `mcp__wraith-browser__browse_search` with `{ "query": "senior rust" }` (filtered against the cached snapshot) |
| 45.0 | Claude's prose continues, now grounded in the data: *"L3Harris has 3 senior Rust openings — 2 in Melbourne FL require an active Secret clearance, 1 in Salt Lake City requires Top Secret/SCI…"* |
| 50.0 | Caption appears |
| 58.0 | End scene |

**Caption:**

> **Same browser. Now driven by an LLM.**
> 130 MCP tools. Zero glue code. Works with Claude Code, Cursor, any MCP client.

**Why this scene here:** Up to this point a smart viewer could think "ok this is just a faster scraper." Scene 4 reframes the entire product: the same engine that just did 717 jobs in 1.83s is **the same engine an agent uses inside a conversation turn**. That collapses two product categories (scraping infra + browser-use agents) into one binary. This is the actual moat.

---

### SCENE 5 — Outro card (0:58 – 1:15)

**Duration:** 17s
**Layout:** Static card, neutral background matching foss-site (`fd-background` ≈ `#0a0a0a`), emerald accents (`#10b981`).

**Card layout (centered, vertical stack):**

```
                    WRAITH BROWSER
        the browser your AI agent deserves

        wraith-browser.vercel.app
        github.com/suhteevah/wraith-browser

           Free during beta · AGPL-3.0
```

**Animation:**
- Logo / wordmark fades in at 58.0s
- Tagline fades in at 59.0s
- URLs fade in at 60.0s, with a subtle emerald underline animation
- "Free during beta" fades in last at 62.0s with a soft glow

Hold the static frame from 62s → 75s. **Do not** add a "subscribe" button, "follow on Twitter," or social icons. The two URLs are the only CTAs. Anyone who cares enough to act will type one of them.

**Why this scene here:** The video has done its job by 58s. The remaining 17 seconds are a *static URL screen* so that when the autoplay loop restarts on Twitter, the viewer who just watched it has the URL on screen long enough to actually navigate. Most product demos blow this — they outro with motion graphics that obscure the only thing that matters.

---

### Total: 75 seconds. Well inside the 60–90s envelope.

---

## Part 2 — Pre-Record Checklist & Production Notes

### Pre-record setup

#### Terminal & shell

- [ ] **Terminal:** Use Windows Terminal (NOT Git Bash directly — looks dated). Tab profile = `PowerShell` running `bash` for the demo session. Or just iTerm2 in a Mac VM if you want the cleanest look.
- [ ] **Font:** JetBrains Mono 18pt (large enough to read on a phone). Bold weight for prompt char.
- [ ] **Colors:** Background `#0a0a0a`, foreground `#e5e5e5`, accent `#10b981` (matches foss-site emerald). Red-pane variant for Playwright: accent `#ef4444`.
- [ ] **Prompt:** Strip it. Set `PS1='$ '` so the prompt is just `$ ` — no hostname, no path, no git branch, no timestamp. Every character on screen must be *content*.
- [ ] **History:** `unset HISTFILE` so up-arrow doesn't leak prior sessions on screen.
- [ ] **Window:** 1920×1080 with a 60px margin all around. The terminal pane is 1800×960. This leaves bleed room for the lower-third caption overlay without the caption ever covering text.

#### Tokens & API

- [ ] **Bootstrap a fresh org for the recording** so the JWT in screenshots is throwaway:
  ```bash
  curl -sS https://wraith-browser.vercel.app/api/v1/auth/register \
    -H 'content-type: application/json' \
    -d '{"email":"demo@ridgecellrepair.com","password":"<long-random>","org_name":"demo","display_name":"demo"}'
  ```
  Note: the `display_name` field is **required** — known bug, see HANDOFF 2026-05-01 §"Bugs".
- [ ] Export `TOK` and `SID` (a pre-created session id) to environment so the on-screen commands are short. The commands shown in the script use `$TOK` and `$SID` — these will render literally if the env vars are set, keeping the visible line short. **Verify before each take that the variables are set in the recording shell.**
- [ ] **Pre-create the session** before hitting record so Scene 1 doesn't include the session-create round trip:
  ```bash
  SID=$(curl -sX POST .../sessions -H "Authorization: Bearer $TOK" -d '{"engine":"native"}' | jq -r .id)
  ```
- [ ] Test all 4 commands end-to-end **30 minutes before recording** to confirm the VPS is warm (cold-start adds ~400ms to first request).

#### Screen capture

- [ ] **Resolution:** Record at 2560×1440 (record big, downscale on export — sharper text).
- [ ] **Frame rate:** 60fps. Terminals look bad at 30fps when text scrolls.
- [ ] **OBS settings:**
  - Output: 2560×1440 → downscale to 1920×1080 in the canvas, lanczos filter
  - Encoder: NVENC HEVC, CQP 18, 2-pass off (single-pass for speed)
  - Color space: sRGB, full range
  - Audio: **disabled at the source level** so there's zero risk of ambient noise leaking
- [ ] **Cursor:** Use a custom blinking block cursor at 1.2× normal size. Default cursors are invisible at 1080p downscale.

#### Live API failure backup plan

The Vercel proxy has a 30s timeout and the VPS is single-tenant shared with TRW — it can stall. **Pre-record every segment in isolation as a fallback.**

- [ ] For each terminal scene, capture both **a live take** and **a pre-recorded asciinema cast** of the same command with known-good output.
- [ ] If a live take fails, splice in the asciinema cast. Use [agg](https://github.com/asciinema/agg) to convert `.cast` → `.gif` → import as video layer in DaVinci Resolve.
- [ ] **Always capture a "JSON has been pre-saved to disk" version** of each command:
  ```bash
  alias demo-boeing='cat ~/.demo-cache/boeing.json | jq'
  ```
  If the live API dies on recording day, swap `curl` for `cat` and re-record. The visual output is identical.

#### Tools to use

| Tool | Purpose | Why this one |
|---|---|---|
| **OBS Studio** | Screen capture, split-screen scenes | Free, scriptable, deterministic |
| **asciinema** | Backup terminal capture | Lossless replay; never drops a frame |
| **agg** | asciinema → gif/video | Pixel-perfect renders for splicing |
| **DaVinci Resolve (Free)** | Final cut, captions, color, export | Free, 60fps support, Fusion for caption animation |
| **ScreenStudio (Mac) or Cleanshot X (Mac)** | If recording on Mac, use this *instead of* OBS for cinematic auto-zoom on cursor | Matches what people expect from polished SaaS demos in 2026 |
| **ffmpeg** | Final encode, aspect-ratio variants | The only deterministic way to produce 1:1 / 9:16 crops |
| **Inter** (font) | Captions / lower-thirds | Matches Vercel-era SaaS aesthetic; renders well at all sizes |

#### Audio decision (silent demo) — DO NOT SKIP

This is **silent by design**, not by laziness. Document it on the YouTube description and the embed page. Reasoning:

- Twitter, LinkedIn, and Reels autoplay muted. A silent demo with captions plays at 100% effectiveness in the muted feed; a voiceover demo plays at 0%.
- HN comment threads watch on conference Wi-Fi or in coffee shops. Anything with audio loses half the audience to the volume button.
- A narrated demo signals "marketing video." A silent terminal demo signals "engineer made this for engineers." That signal alone gets it past the HN BS filter.
- It's faster to produce. No script revisions for VO timing. No re-records when the narrator misreads a line. No paying for a voice actor.

If you ever want voice for a longer-form YouTube version, record the silent cut first and add VO as a separate track over the same video.

#### Asset list

| Asset | Purpose | Spec |
|---|---|---|
| `wraith-wordmark.svg` | Outro card logo | Vector, white on transparent |
| `JetBrainsMono-Bold.ttf` | Terminal font | Self-host in OBS scene |
| `Inter-Bold.ttf`, `Inter-Regular.ttf` | Captions | 48pt bold for headline, 28pt regular for sub |
| `caption-template.psd` | Lower-third template | Bottom 180px, 80% black bg with 4px emerald top border |
| `outro-card.png` | Static card frame | 1920×1080 + 1080×1080 + 1080×1920 variants |
| `cursor-block.png` | Custom cursor overlay | 24×40px emerald block, 1Hz blink |

#### Color palette (matches foss-site)

| Use | Hex | Notes |
|---|---|---|
| Background | `#0a0a0a` | Matches `--fd-background` (neutral) |
| Primary text | `#e5e5e5` | Slight off-white, easier on eye |
| Accent (Wraith) | `#10b981` | `emerald-500` from Tailwind, matches page.tsx CTAs |
| Accent dim (Wraith) | `#34d399` | `emerald-400`, used in foss-site for hover states |
| Contrast (Playwright pane only) | `#ef4444` | `red-500` — only ever appears in Scene 1 |
| Caption background | `rgba(0,0,0,0.8)` | Sits above terminal without obscuring |
| Caption headline | `#ffffff` | Bold |
| Caption sub-line | `#a1a1aa` | `zinc-400` |

---

### Distribution targets & aspect ratios

| Channel | Aspect | Resolution | Notes |
|---|---|---|---|
| Landing-page embed (foss-site `/` hero) | 16:9 | 1920×1080 | Autoplay, muted, loop. Poster frame = Scene 5 outro. |
| YouTube | 16:9 | 1920×1080, 60fps | Same master. Add YouTube card linking to `wraith-browser.vercel.app` at 60s. |
| Twitter / X feed | 1:1 | 1080×1080 | Crop centered. Verify nothing gets cut from the split-screen — use a 90% safe area. |
| LinkedIn feed | 1:1 | 1080×1080 | Same 1:1 export reused. |
| TikTok / Reels / Shorts | 9:16 | 1080×1920 | **Reframe, don't crop.** Stack the split-screen vertically (Playwright top, Wraith bottom). Use ffmpeg's `vstack` filter. The vertical version is essentially a different edit — budget time for it. |
| HN post | (link to YouTube) | n/a | The HN post is text. Link to the YouTube 16:9. Don't try to embed video. |

**FFmpeg recipes:**

```bash
# 1:1 export (center crop)
ffmpeg -i master_1080p.mp4 -vf "crop=1080:1080:420:0" -c:v libx264 -crf 18 -c:a copy wraith_1x1.mp4

# 9:16 export — requires a separate vertically-stacked re-edit for Scene 1.
# For Scenes 2–5 (single-pane), just pad:
ffmpeg -i master_1080p.mp4 -vf "scale=1080:-1,pad=1080:1920:0:(1920-ih)/2:black" -c:v libx264 -crf 18 wraith_9x16.mp4
```

---

### Production timeline estimate

| Phase | Time |
|---|---|
| Pre-flight: terminal setup, token bootstrap, dry-runs of all 4 commands | 1.5h |
| Recording: each scene × 3 takes minimum (live + 2 backup) | 2h |
| Editing: cut, captions, transitions, color match | 3h |
| Aspect-ratio variants (1:1 + 9:16 reframe) | 1.5h |
| Final encodes, file delivery, embed on landing page | 1h |
| **Total estimated production time** | **~9 hours** (one focused day) |

If a producer is doing this from scratch with no prior Wraith context, add 1.5h for them to read CAPABILITIES.md and the corpo wiki page. **Total cold-start: ~10.5 hours.**

---

### Sanity checks before publishing

- [ ] All on-screen JWTs / session IDs are from the throwaway demo org and have been **revoked** before the video goes public
- [ ] No prompt leaks the home directory path (`~/projects/wraith-browser` is fine; `C:/Users/Matt/...` is not)
- [ ] Captions are spell-checked and the URL in the outro is **typed twice and verified clickable** (broken URL on a launch demo is the single most common embarrassing mistake)
- [ ] Embed on `foss-site/app/page.tsx` uses `<video muted autoplay loop playsinline poster="...">` so it works on iOS Safari
- [ ] The 9:16 cut has been viewed on an actual phone, not just in the editor — vertical layouts always look different at the device
- [ ] The HN submission is scheduled for Tuesday or Wednesday 9am Pacific (highest dwell time for technical posts)
