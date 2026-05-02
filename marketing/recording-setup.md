# Recording setup — Kokonoe (Win10, RTX 3070 Ti)

Just-press-record companion to `marketing/demo-script.md`. The script tells you
*what* to record; this tells you exactly *how* on Kokonoe so a recording
session is ~30 minutes of actual work, not a half-day of fighting OBS.

> **Demo runs entirely off `https://wraith-browser.vercel.app` and
> `http://207.244.232.227:8080`.** No local Wraith build needed for the
> demo itself — the binary on pixie is what's getting demoed.

---

## 1. One-time setup (~15 minutes)

### Install

```powershell
# OBS — recording
choco install obs-studio -y

# ffmpeg — post-prod splicing, format reframes
choco install ffmpeg -y

# jq — JSON pretty-print in the demo terminal
choco install jq -y
```

(All four already exist in standard chocolatey. UAC is disabled per global
CLAUDE.md so no prompts.)

### Windows display

Settings → System → Display:
- **Resolution:** 2560×1440 (native on Kokonoe)
- **Scale:** 100% (Windows scaling above 100% blurs OBS captures — cap at native)
- **Night light:** OFF for the recording (color shifts ruin the captures)

### Windows Terminal profile (the demo terminal)

Open Windows Terminal → Settings → "Open JSON file". Add a new profile:

```jsonc
{
  "name": "Wraith Demo",
  "commandline": "powershell.exe -NoLogo -NoProfile -Command \"$env:PS1='$ '; pwsh -NoLogo\"",
  "fontFace": "JetBrains Mono",
  "fontSize": 18,
  "fontWeight": "normal",
  "padding": "24",
  "useAcrylic": false,
  "backgroundOpacity": 1.0,
  "background": "#0d1117",
  "foreground": "#e6edf3",
  "cursorColor": "#10b981",
  "cursorShape": "filledBox",
  "colorScheme": "Wraith Demo",
  "antialiasingMode": "cleartype",
  "scrollbarState": "hidden"
}
```

And add this colorScheme block (under `"schemes"`):

```jsonc
{
  "name": "Wraith Demo",
  "background": "#0d1117",
  "foreground": "#e6edf3",
  "black":   "#484f58",
  "red":     "#ff7b72",
  "green":   "#10b981",
  "yellow":  "#d29922",
  "blue":    "#58a6ff",
  "purple":  "#bc8cff",
  "cyan":    "#39c5cf",
  "white":   "#b1bac4",
  "brightBlack":  "#6e7681",
  "brightRed":    "#ffa198",
  "brightGreen":  "#56d364",
  "brightYellow": "#e3b341",
  "brightBlue":   "#79c0ff",
  "brightPurple": "#d2a8ff",
  "brightCyan":   "#56d4dd",
  "brightWhite":  "#ffffff"
}
```

Background `#0d1117` and accent `#10b981` match the foss-site Tailwind dark
theme — captures will look like the live site.

### OBS scenes

Settings → Output → Recording:
- **Type:** Standard
- **Recording Path:** `J:\wraith-browser\marketing\recordings\` (mkdir first)
- **Recording Format:** mkv (won't corrupt mid-record if OBS crashes)
- **Encoder:** NVIDIA NVENC H.264 (your 3070 Ti has it, free CPU for everything else)
- **Rate Control:** CQP, CQ Level 18 (visually lossless)
- **Preset:** P5 (Quality)
- **Profile:** high
- **Look-ahead:** on
- **Psycho-visual tuning:** on

Settings → Video:
- **Base (Canvas) Resolution:** 2560×1440
- **Output (Scaled) Resolution:** 2560×1440 (downscale happens in ffmpeg, not OBS)
- **FPS:** 60

Settings → Audio:
- **Disable everything** — this is a silent demo per the script. Mute mic + desktop. Verify the recording has zero audio tracks before exporting; nothing is worse than accidental keyboard click sounds in the master.

Create three OBS scenes:

1. **`split-screen`** — two Display Capture sources side by side.
   - Left: 1280×1440 crop showing terminal #1 (Playwright)
   - Right: 1280×1440 crop showing terminal #2 (Wraith)
   - Add a 4px vertical divider (Color Source, `#21262d`)

2. **`single-terminal`** — one Display Capture, full canvas, cropped to a 2400×1200 centered region (leaves margin for caption lower-third).

3. **`outro`** — Image Source pointing at `marketing/assets/outro-card.png`.
   - Render the outro card from the script as a static 2560×1440 PNG. (See `assets/` section below.)

Captions go in via OBS Text (GDI+) source, font JetBrains Mono 36pt
`#e6edf3` on a 60% opaque `#0d1117` rectangle — toggle visibility with hotkeys
during the take. Don't try to time-sync them in post; that's a rabbit hole.

### Asset prep

```powershell
mkdir J:\wraith-browser\marketing\assets
mkdir J:\wraith-browser\marketing\recordings
mkdir J:\wraith-browser\marketing\final
```

Make `assets/outro-card.png` (2560×1440, dark `#0d1117` background) with three
text blocks:

```
WRAITH BROWSER

https://wraith-browser.vercel.app
github.com/suhteevah/wraith-browser

Free during beta · AGPL-3.0
```

Any tool — I'd use a quick Figma frame, but Powerpoint export to PNG also works.

### One-time API bootstrap (do this BEFORE recording)

So you have a working JWT for the live demo and don't burn a take on the
register call:

```powershell
$env:WRAITH_BASE = "https://wraith-browser.vercel.app"
curl.exe -X POST "$env:WRAITH_BASE/api/v1/auth/register" `
  -H "content-type: application/json" `
  -d '{"email":"demo@ridgecellrepair.com","password":"<long-random>","org_name":"Wraith Demo","display_name":"Demo"}' `
  | jq -r .access_token > $env:USERPROFILE\.wraith-demo-token

# Verify
$env:WRAITH_TOKEN = Get-Content $env:USERPROFILE\.wraith-demo-token
curl.exe -H "Authorization: Bearer $env:WRAITH_TOKEN" "$env:WRAITH_BASE/api/v1/auth/me"
```

Register sends `display_name` explicitly per `NEXT-UP.md` BR-5. The token is
good for 1h; refresh between takes if needed via `/api/v1/auth/login`.

---

## 2. Pre-cache the demo responses (failure recovery)

Per the demo-script's recovery section: pre-record every API response so a
flaky upstream during a take doesn't cost a retake. Cache once, alias the
playback in the demo shell.

```powershell
mkdir $env:USERPROFILE\.wraith-demo-cache
$h = "Authorization: Bearer $env:WRAITH_TOKEN"

# Boeing — Radancy
curl.exe -sS -H $h -X POST "$env:WRAITH_BASE/api/v1/sessions" `
  -d '{"target":"https://jobs.boeing.com"}' `
  > $env:USERPROFILE\.wraith-demo-cache\boeing-session.json

# (Repeat for L3Harris, Lockheed, MITRE per the script — same shape)

# Swarm fan-out result (4-tenant, jobs across all platforms)
curl.exe -sS -H $h -X POST "$env:WRAITH_BASE/api/v1/swarm/fan-out" `
  -d '{"urls":["https://jobs.boeing.com","https://careers.l3harris.com","https://lockheedmartinjobs.com","https://careers.mitre.org"],"action":"hydrate"}' `
  > $env:USERPROFILE\.wraith-demo-cache\swarm.json
```

In the demo shell, alias the live curl to the cache when you want a guaranteed
clean run:

```powershell
function demo-boeing { Get-Content $env:USERPROFILE\.wraith-demo-cache\boeing-session.json | jq . }
```

Use the live curl for the hero take (the wall-clock matters there). Use the
alias for repeats and the swarm scene where 4× simultaneous timing is hard
to keep clean.

---

## 3. Recording-day pre-flight (~5 minutes)

In order, ten boxes to check before hitting record:

- [ ] Close Slack, Discord, Telegram desktop, mail clients (notification bubbles ruin takes)
- [ ] Disable Windows toast notifications: Settings → System → Notifications → Off
- [ ] Set Windows wallpaper to solid `#0d1117` (Settings → Personalization → Background → Solid color)
- [ ] Hide taskbar: right-click taskbar → Taskbar settings → "Automatically hide the taskbar"
- [ ] Browser: open the live site at `https://wraith-browser.vercel.app/vs` in a fresh window, F11 fullscreen, dark mode (already default)
- [ ] Two Windows Terminal windows in `Wraith Demo` profile, positioned for the split-screen OBS scene crop
- [ ] OBS: hit "Studio Mode" so you can check shots without going live; verify all 3 scenes preview correctly; verify audio meters are dead silent (mic muted, desktop muted)
- [ ] Refresh the JWT (one curl): `Get-Content` should be a 64-char access_token blob; `/api/v1/auth/me` returns 200
- [ ] Run each script scene once on the timer. If the wall-clock isn't comparable to the script's claims, swap to the cached version.
- [ ] Final dry run of the whole 75 seconds with a stopwatch. If it's >85s, cut a beat. If <65s, hold the outro longer.

---

## 4. Recording

Per the script: 5 scenes, ~75 seconds total. Record each scene as its own
OBS take to its own mkv, then assemble in ffmpeg. Don't try to do it in one
unbroken take — retakes get cheap when each scene is independent.

Naming convention:
```
recordings/scene1-splitscreen-take03.mkv
recordings/scene2-jsoninspect-take01.mkv
recordings/scene3-swarm-take02.mkv
recordings/scene4-mcp-take04.mkv
recordings/scene5-outro-take01.mkv
```

`take01` is your first cut; `take02..N` are retakes. Pick the best take of
each scene by inspection, then assemble.

---

## 5. Post-prod ffmpeg

### Assemble the 16:9 master (1080p)

```powershell
cd J:\wraith-browser\marketing

# Concat the chosen takes. Edit BEST.txt to point at the take you picked
# for each scene.
@'
file 'recordings/scene1-splitscreen-take03.mkv'
file 'recordings/scene2-jsoninspect-take01.mkv'
file 'recordings/scene3-swarm-take02.mkv'
file 'recordings/scene4-mcp-take04.mkv'
file 'recordings/scene5-outro-take01.mkv'
'@ | Out-File -Encoding ASCII recordings/best.txt

# Concat losslessly (no re-encode)
ffmpeg -f concat -safe 0 -i recordings/best.txt `
  -c copy recordings/master-1440p.mkv -y

# Downscale + h264 + AAC-silence to a web-friendly mp4
ffmpeg -i recordings/master-1440p.mkv `
  -vf "scale=1920:1080:flags=lanczos" `
  -c:v libx264 -preset slow -crf 18 -pix_fmt yuv420p `
  -movflags +faststart -an `
  final/wraith-demo-1080p.mp4 -y
```

**Why -an** (no audio): forces a clean audio-less stream; some platforms
auto-mute when an audio track is detected as silent.

### 1:1 square (Twitter/LinkedIn feed) — 1080×1080

The split-screen scene 1 is 16:9; cropping to 1:1 would lose half the comparison.
Re-frame as a vertical stack instead:

```powershell
# Split scene 1 into left/right halves and stack them vertically
ffmpeg -i recordings/scene1-splitscreen-take03.mkv `
  -filter_complex "[0:v]crop=iw/2:ih:0:0[L];[0:v]crop=iw/2:ih:iw/2:0[R];[L][R]vstack=inputs=2[v]" `
  -map "[v]" -c:v libx264 -preset slow -crf 18 -an `
  recordings/scene1-stacked.mkv -y

# Now build the 1:1 master with stacked scene 1
@'
file 'recordings/scene1-stacked.mkv'
file 'recordings/scene2-jsoninspect-take01.mkv'
file 'recordings/scene3-swarm-take02.mkv'
file 'recordings/scene4-mcp-take04.mkv'
file 'recordings/scene5-outro-take01.mkv'
'@ | Out-File -Encoding ASCII recordings/best-square.txt

ffmpeg -f concat -safe 0 -i recordings/best-square.txt `
  -vf "scale=1080:-2:flags=lanczos,crop=1080:1080" `
  -c:v libx264 -preset slow -crf 18 -pix_fmt yuv420p `
  -movflags +faststart -an `
  final/wraith-demo-square.mp4 -y
```

### 9:16 vertical (Shorts/Reels/TikTok) — 1080×1920

```powershell
# Same vstack trick for scene 1, then crop to 9:16 throughout
ffmpeg -f concat -safe 0 -i recordings/best-square.txt `
  -vf "scale=1080:-2:flags=lanczos,crop=1080:1920" `
  -c:v libx264 -preset slow -crf 18 -pix_fmt yuv420p `
  -movflags +faststart -an `
  final/wraith-demo-vertical.mp4 -y
```

(The crop will trim a band off the captions — re-render captions in OBS
specifically for the 9:16 take if you want them perfect. For an internal
launch, the band-cropped version is fine.)

### Verify

```powershell
ffprobe -v error -show_entries stream=width,height,codec_name,duration `
  -of default=nw=1 final/wraith-demo-1080p.mp4
ffprobe -v error -show_entries stream=width,height,codec_name,duration `
  -of default=nw=1 final/wraith-demo-square.mp4
ffprobe -v error -show_entries stream=width,height,codec_name,duration `
  -of default=nw=1 final/wraith-demo-vertical.mp4
```

Expected: 1920×1080 ~75s, 1080×1080 ~75s, 1080×1920 ~75s, all h264, no audio
streams.

### File sizes

At CRF 18, expect ~25-50 MB per output. If a target needs <8 MB (some Discord
servers, some embed contexts), bump to CRF 23 and add `-tune film`.

---

## 6. Upload targets

| Surface | File | Notes |
|---|---|---|
| Landing page hero embed | `wraith-demo-1080p.mp4` | `<video autoplay muted loop playsinline>` — the muted + playsinline are required for iOS auto-play |
| HN post (no inline video, link out) | host on Vercel as a static file at `foss-site/public/demo.mp4`, link in the post | Vercel serves static MP4s with proper Range support |
| Twitter/X primary post | `wraith-demo-square.mp4` | 1:1 plays at full size in feeds; 16:9 letterboxes |
| LinkedIn | `wraith-demo-square.mp4` | Same reasoning |
| YouTube (channel home + Shorts) | `wraith-demo-1080p.mp4` (full) + `wraith-demo-vertical.mp4` (Shorts) | Shorts max 60s — trim if needed |
| TikTok / IG Reels | `wraith-demo-vertical.mp4` | Already 9:16 |

Drop them in `J:\wraith-browser\foss-site\public\` and re-deploy to expose at
`https://wraith-browser.vercel.app/demo.mp4` etc.

---

## 7. Rough wall-clock budget

| Step | Time |
|---|---|
| One-time setup (this doc, top to "Recording") | 15 min |
| Pre-cache responses + JWT bootstrap | 5 min |
| Pre-flight + dry run | 10 min |
| Takes (5 scenes × ~3 takes avg) | 30-45 min |
| Pick best, assemble, post-prod 3 outputs | 20 min |
| Upload + foss-site deploy | 10 min |
| **Total** | **~1.5-2 hours** |

(The script estimated 9-10h for a producer cold-starting; on Kokonoe with this
checklist, it should be the lower number. If your first session goes long,
the second one halves.)

---

## 8. Known traps

- **Windows Terminal anti-aliasing** flickers between frames if `antialiasingMode` is set to `aliased`. Stick with `cleartype`.
- **OBS NVENC** silently falls back to x264 if another app is using NVENC (e.g., a Discord screen-share). Check Settings → Output and confirm "NVIDIA NVENC H.264" is selected before each session.
- **The Vercel rewrite has a ~30s timeout** — long demo curls (a slow swarm fan-out hit) will 504. The script avoids this by using cached responses for the swarm scene; if you record the swarm scene live and it hangs, that's why.
- **Cloudflare on Boeing/RTX** can intermittently flag the stealth fingerprint and bounce a take. Pre-cached responses are the workaround; that's why you pre-cached.
- **The demo JWT will expire 1h after register.** If a session bleeds long, refresh via `/auth/login` between takes — don't burn 5 minutes mid-shoot debugging "why is the API returning 401."
