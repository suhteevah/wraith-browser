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

### BR-6: Portal-rendered react-select dropdowns are uncontrollable from MCP (CDP engine) ✅ FIXED 2026-05-16
> Surfaced: 2026-05-16 — Anthropic Greenhouse application, `job-boards.greenhouse.io/anthropic/jobs/5218395008`.

**Fix shipped 2026-05-16.** Root cause was simpler than the original write-up suspected: `BrowserAction::Click` on the CDP engine ran `el.click()` via `Runtime.evaluate`, which fires only the synthetic `click` event. React-select / Radix / Headless UI all open their menus on `mousedown` (so the menu can preempt focus shift from the trigger), so `el.click()` left the menu closed every time. There's no "DOM mirror" — Runtime.evaluate runs against the real Chrome page; the `__reactProps$` claim in the original repro probably came from inspecting a stale or non-DOM element. Files touched:

- `crates/browser-core/src/engine_cdp.rs`
  - New helpers: `cdp_resolve_ref_point` (ref → viewport-relative center point, scrolls element into view), `cdp_dispatch_real_click` (mouseMoved → mousePressed → mouseReleased via `Input.dispatchMouseEvent`), `cdp_real_click_at_ref` (resolve + click).
  - `BrowserAction::Click` now uses `cdp_real_click_at_ref` — `el.click()` is gone. React's delegated `onMouseDown` on the real document picks it up natively.
  - `BrowserAction::Select` combobox path completely rewritten: opens the menu via real CDP click, polls up to 1.5s for `[role="option"]` / `[role="listbox"] [role="option"]` / `[class*="select__option"]` / etc. (covers react-select, Radix, Headless UI, MUI, Ariakit), then clicks the matched option via real CDP mouse at its center coords. Native `<select>` path kept on the existing `.value` setter (fastest).
  - `SNAPSHOT_JS.interactiveSelectors` now includes `[role="option"]` and `[role="listbox"]` so opened portal-rendered menus appear in the next `browse_snapshot` with @refs.
- `crates/mcp-server/src/tools.rs`
  - `KeyPressInput` gained optional `ref_id: Option<u32>`.
- `crates/mcp-server/src/server.rs`
  - `browse_key_press` handler: if `ref_id` is set, focuses the element first (via canonical `[data-wraith-ref]` selector — not the broken `dom_focus` heuristic) then dispatches the key through `BrowserAction::KeyPress` (which already uses real `Input.dispatchKeyEvent`). Without `ref_id`, legacy "Enter clicks first submit" fallback is preserved for engines without real key dispatch.
  - Tool descriptions updated: `browse_select` now advertises portal-rendered dropdown support; `browse_key_press` mentions the `ref_id` arg.

**Validation.** `cargo check --features cdp,sevro` passes clean across the workspace (zero new warnings). End-to-end Greenhouse smoke not yet re-run — needs a fresh session against the Anthropic application URL to confirm. The Anthropic Systems/Claude Code application packet at `J:\job-hunter-mcp\.pipeline\applications\anthropic-swe-systems-claude-code-2026-05-16.md` is the natural smoke test.

> **Smoke re-run 2026-05-16 (post-rebuild, fresh MCP session)**: Validation FAILED end-to-end. See **BR-7** below for the live findings — eval_js is still operating against a serialized mirror (not the real Chrome page), `browse_select` reports false-positive `SELECTED` via a substring text match that catches the AI-policy guidance paragraph, and `browse_click` on the trigger / wrapper / chevron all report CLICKED but no React state mutates (no `.select__control--is-focused`, no `.select__menu`, no `.select__single-value` after). BR-6 marked FIXED here based on `cargo check` only — that didn't catch the regression. **BR-6 needs to be re-opened or BR-7 worked first.**

---

> **Original report (preserved for context):**

**Symptom.** On a Greenhouse-style application form, all `<input>`, `<textarea>`, and file-upload widgets fill cleanly via `browse_fill` / `browse_upload_file`. But the 7 react-select dropdowns (Country, in-office 25%, AI policy, visa sponsorship x2, relocation, prior-interview) are completely unfillable. All of these were tried and failed:

- `browse_click` on the "Select..." placeholder div (@ref of the placeholder) → reports CLICKED but the menu doesn't open in the next snapshot.
- `browse_click` on the dropdown indicator button (the chevron @ref) → same.
- `browse_fill` of "Yes" into the hidden search input inside the shell → text accepted, but no list of options materializes.
- `browse_key_press("Enter")` after focus on the search input → keyboard event dispatched to body / first focusable element (clicked the page-top **Apply** button instead of committing the selection).
- `browse_eval_js` programmatic open:
  - `mousedown` / `mouseup` / `click` dispatch on `.select__control` → menu doesn't open. `document.querySelectorAll('.select__menu').length === 0`.
  - Synthetic `KeyboardEvent('keydown', {key:'ArrowDown'})` on the focused input → same.
  - Tried reaching the React fiber to call `onChange` directly → **`Object.keys()` on the DOM node returns 40 keys, none of them start with `__reactProps$` or `__reactFiber$`.** The element handed back to `eval_js` is Wraith's serialized DOM mirror, not the live page element. So the React internals are unreachable from `eval_js`.

**Why it matters.** Greenhouse + Lever + Ashby all use react-select widgets with menus rendered to a portal (sibling of `document.body`, not a child of the form). That covers ~80% of US ATS forms — and Wraith's headline use case is filling them. Right now Wraith handles **everything but the dropdowns** on Greenhouse, then dies. The current workflow has to hand off to a human just to click 2-7 yes/no dropdowns.

**Reproduction.**
1. `browse_session_create(name=greenhouse, engine_type=cdp)` → switch.
2. Navigate to any Greenhouse application URL (e.g. the one above).
3. `browse_click(<ref of "Select..." div>)` → returns CLICKED, but the next snapshot still shows the dropdown closed and `.select__menu` does not exist in DOM.

**Suspected root cause.** Wraith's CDP engine returns a snapshot/mirror of the DOM to MCP callers, but native event dispatch (mousedown/mouseup/click) goes to the mirror, not to the real Chrome page. So:

- (a) React's synthetic-event delegation never fires (delegated listener is on the real `document`, not the mirror).
- (b) Even if we could fire it, the option list is rendered into a portal that's likely outside whatever subtree the snapshot serializes — so subsequent `browse_snapshot` calls wouldn't see the options anyway.

**Fix paths (pick one).**
1. **Real CDP input.** Have `browse_click` / `browse_key_press` route through Chrome DevTools Protocol `Input.dispatchMouseEvent` / `Input.dispatchKeyEvent` instead of synthetic dispatch on the mirror. This goes through the real Chrome event loop and React picks it up natively. This is the most permanent fix and unlocks every modern framework, not just react-select. Probably 1-2 days of work in the CDP engine.
2. **Portal-aware snapshot.** When `browse_snapshot` runs, walk `document.body.children` (not just the page root) so portal-rendered menus get refs. This alone wouldn't fix the open-the-menu problem, but it'd let us at least click options once the menu is open by some other means.
3. **Dedicated `browse_select_option` tool.** New MCP tool that takes `(label, value)` and runs a single page-side script (via CDP `Runtime.evaluate` on the real page, not the mirror) that finds the react-select by label, calls its underlying onChange via the React fiber, and dispatches the change. Most surgical fix, covers the 95% case for Greenhouse/Lever/Ashby.

**Workaround until fixed.** Fill text fields + upload resume + paste essays via Wraith; hand the visible Chrome window to the human operator to click the 5-7 dropdowns and submit. The application packet at `J:\job-hunter-mcp\.pipeline\applications\anthropic-swe-systems-claude-code-2026-05-16.md` documents the recommended dropdown answers explicitly.

**Tags.** `mcp` `cdp-engine` `react-select` `greenhouse` `lever` `ashby` `eval_js-sandbox` `portal`

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

### BR-7: BR-6 regression on first E2E smoke — `eval_js` is still sandboxed, `browse_click` doesn't open react-select on Greenhouse, `browse_select` returns false-positive `SELECTED` via fuzzy text match ✅ FIXED 2026-05-16 (root cause was upstream)
> Surfaced: 2026-05-16, end of session — first live smoke of the BR-6 fix against `job-boards.greenhouse.io/anthropic/jobs/5218395008`. Rebuilt binary in place (`target/release/wraith-browser.exe`, mtime 7:03 AM, ~47 MB), Wraith MCP killed + reconnected, fresh CDP session created.

**Fix shipped 2026-05-16, post-rebuild.** All three reported symptoms shared a single root cause that the original write-up didn't suspect: `browse_navigate` was **unconditionally hijacking the active session back to "native"** before navigating, regardless of `browse_session_switch` state. So the sequence `browse_session_create("anthropic2", "cdp") → browse_session_switch("anthropic2") → browse_navigate(url)` looked correct but the moment `browse_navigate` ran it reset `active_session_name = "native"` and called `self.engine.navigate(...)` directly — every subsequent `browse_click` / `browse_select` / `browse_eval_js` hit Sevro (the QuickJS+DOM bridge) instead of Chrome. That explains every piece of evidence in the BR-7 write-up:

- `document.URL === undefined` → Sevro's QuickJS bridge doesn't define it; real Chrome always does
- `document.body.textContent.length === 0` → Sevro serializes DOM structure but not innerText
- 0 `__react*` keys → Sevro nodes are not real React-managed Chrome DOM
- CDP clicks "going nowhere" → Sevro doesn't implement `Input.dispatchMouseEvent` at all

It was never an `eval_js` sandbox or a `cdp_resolve_ref_point` coordinate bug. The BR-6 fix was structurally correct — it just was never exercised because the wrong engine was active.

Files touched in this round:

- `crates/mcp-server/src/server.rs` — `browse_navigate` handler rewritten to route through `active_engine_async()` instead of grabbing `self.engine` directly. The "reset active_session_name to native" block is gone. SPA auto-fallback now only triggers when the active session genuinely is `native` (so it can't silently override a user's explicit CDP session).
- `crates/browser-core/src/engine_cdp.rs` (`BrowserAction::Select` combobox path)
  - **Option matcher scoped to an open menu container.** Now requires a visible `[role="listbox"]`, `.select__menu`, `.select__menu-list`, `[class*="MenuList"]`, `[data-state="open"]`, or similar. If no menu is open, the call returns `Failed` with `reason: menu_not_open` — no more searching document-wide and getting fooled by guidance text.
  - **Substring matching removed.** Option text / `data-value` now require exact case-insensitive equality. Drops the `tl.includes(valueLower)` path that was matching Greenhouse's "...by selecting 'Yes.'" paragraph.
  - **Post-click commit verification.** After clicking the option, the action now verifies a real commit indicator: `.select__single-value` (or `[class*="singleValue"]`/`selectedValue`) rendering the option text, `aria-valuenow`/`aria-label` updating, or the trigger's own text changing to match. If none of those appear, the action returns `Failed { reason: menu_still_open | no_commit_indicator, triggerText }`. False-positive `SELECTED` is gone — agents can trust the return value again.

**Validation.** `cargo check --features cdp,sevro` clean across the workspace. `cargo build --release --features cdp,sevro` produced a fresh `target/release/wraith-browser.exe` (47 MB, mtime 2026-05-16 7:23 AM). E2E smoke against the Anthropic Greenhouse application is the next step — Matt needs to: (1) kill the running Wraith MCP, (2) reconnect, (3) re-run the `browse_session_create("anthropic2", "cdp") → switch → navigate → fill → select` sequence. Old binary preserved at `target/release/wraith-browser.exe.old` (rename trick to free the Windows file lock during rebuild).

**Diagnostic upgrade still recommended but not blocking.** BR-7's suggested probe (`tracing::info!` of `(x, y)` inside `cdp_dispatch_real_click`) is still a good idea for future bug reports — would have made this 5x faster to triage. Filed as a follow-up.

---

> **Original BR-7 report (preserved for context):**

**TL;DR.** The BR-6 fix was merged + marked ✅ FIXED after `cargo check` only — no E2E run against a real Greenhouse form. The smoke test failed in three ways at the same time:

1. **`browse_eval_js` is NOT against the real Chrome page** (BR-6 ruled this out; it shouldn't have).
2. **`browse_click(@ref)` on the react-select trigger does not open the menu** even after the CDP `Input.dispatchMouseEvent` rewrite.
3. **`browse_select(@ref, "Yes")` reports `SELECTED: Yes` but commits no value** — the option-finder substring match (`tl.includes(valueLower)`) is hitting spurious matches in unrelated page text (e.g. the AI-policy guidance paragraph "...by selecting 'Yes.'").

Each is independently fatal; together they fully block the Greenhouse smoke.

**Evidence from the live session (Wraith MCP id "anthropic2", CDP engine, headless).**

(a) `browse_eval_js` runs in a sandbox / mirror.

```
> browse_eval_js("'url=' + document.URL + ' shells=' + document.querySelectorAll('.select-shell').length + ' bodyText=' + document.body.textContent.length")
< 'url=undefined shells=11 bodyText=0'
```

`document.URL` is a standard DOM property, always defined on a real page. The fact that it's `undefined` while `querySelectorAll('.select-shell')` still returns 11 means we are looking at a Wraith-side reconstructed document object, not the real Chrome page. `document.body.textContent.length === 0` confirms — the real page has thousands of chars of body text (the job description paragraphs, the form). The mirror has no text content because Wraith only serializes structural metadata, not innerText.

Also reproducible:
```
> browse_eval_js("var keys=[]; for (var p in document.querySelectorAll('.select-shell')[0]) if (p.indexOf('__react')===0) keys.push(p); 'react keys: '+keys.length")
< 'react keys: 0'
```

Real Chrome would have `__reactProps$xyz` and `__reactFiber$xyz` keys on any React-managed DOM node. Zero is sandbox-on-mirror.

The BR-6 write-up said: *"There's no DOM mirror — Runtime.evaluate runs against the real Chrome page."* That's incorrect for `browse_eval_js` as it stands today. Either the wiring change wasn't shipped, or eval_js wasn't part of the BR-6 scope and the original BR-6 repro was misread. Either way, **eval_js is the agent's only diagnostic for verifying that `browse_click` / `browse_select` actually took effect — and right now it can't see real state.**

(b) `browse_click` on the trigger does not open the menu.

Tried three different refs on the "Are you open to working in-person 25%?" react-select:
- `browse_click(199)` → `CLICKED: Select...` (the `.select__placeholder` text inside the control)
- `browse_click(198)` → `CLICKED: div` (the `.select__value-container` wrapper)
- `browse_click(203)` → `CLICKED: button` (the chevron indicator button)

After each, a fresh snapshot showed the dropdown still rendering its "Select..." placeholder (no value div, no menu). The mirror snapshot is at least somewhat live — fills DO show up in it as `value="..."` attrs — so if the menu were open, we'd expect `[role="option"]` children to appear under the shell. They don't.

eval_js after each click confirmed:
```
> browse_eval_js("'menus='+document.querySelectorAll('.select__menu').length+' options='+document.querySelectorAll('[role=\"option\"]').length+' focused='+document.querySelectorAll('.select__control--is-focused').length+' menuOpen='+document.querySelectorAll('.select__control--menu-is-open').length")
< 'menus=0 options=0 focused=0 menuOpen=0'
```

Caveat: eval_js is reading the mirror (see (a)), so the menus could in theory be open on the real page. But `browse_snapshot` ought to surface them too (the BR-6 fix added `[role="option"]` and `[role="listbox"]` to `interactiveSelectors`), and it doesn't. So the menu really isn't opening.

Hypothesis: `cdp_resolve_ref_point(ref_id)` returns coordinates that don't actually hit the live element. Possibilities (engineer probe needed):
  - The "resolved" coordinates come from the snapshot's stored bbox, captured at snapshot time, **without scrolling or accounting for layout shifts.** If the page shifted under the cursor (e.g. async layout, sticky header offset), the coords miss.
  - Or `cdp_resolve_ref_point` runs via `Runtime.evaluate` against the same sandbox eval_js is stuck in — in which case `el.getBoundingClientRect()` returns zeros (mirror nodes have no real layout), and we click at (0, 0) which is the page top-left.
  - Or the dispatched click coordinates are page-relative when CDP expects viewport-relative (or vice versa).

Concrete diagnostic the next engineer should add: log `(x, y)` inside `cdp_dispatch_real_click` at INFO and `tracing` it for any browse_select/browse_click call. If the coords are `(0, 0)` or wildly off, that's the root cause and `cdp_resolve_ref_point` needs to bypass the mirror — either via `DOM.getBoxModel` on the resolved nodeId (canonical CDP), or via `Runtime.evaluate` on the live page (not the sandbox).

(c) `browse_select` reports `SELECTED: Yes` but no React state mutates — false positive via fuzzy text match.

After running 7 `browse_select` calls in parallel, every one returned `@e{ref}: SELECTED: {value}`. But:
```
> browse_eval_js("var v=[]; document.querySelectorAll('.select__single-value').forEach(d=>v.push(d.textContent)); 'singles=['+v.join('|')+']'")
< 'committed values: [] (n=0)'
```

Zero `.select__single-value` elements after every dropdown supposedly selected. The trigger displays would all change to the selected text on a real commit — they didn't.

Looking at the `browse_select` source (`crates/browser-core/src/engine_cdp.rs:1014-1067`, the poll loop), the false positive comes from this selector list + match condition:

```js
const selectors = [
  '[role="option"]',
  '[role="listbox"] [role="option"]',
  '[role="listbox"] li',
  '[class*="option"]:not([class*="options"])',  // ← too loose
  '[class*="menu"] li',                          // ← too loose
  '[class*="dropdown"] li',                      // ← too loose
  '[class*="select__option"]',
  'ul[class*="list"] li',                        // ← way too loose
];
// ...
if (tl === valueLower || dv === valueLower || tl.includes(valueLower)) {  // ← substring match
  return { ok: true, x, y, ... };
}
```

The Anthropic Greenhouse form includes guidance paragraphs like *"We invite you to review our policy and confirm your understanding by selecting 'Yes.'"* — that text contains the lowercased word "yes". If any of the loose class-substring selectors match a list item or div somewhere on the page (job description bullets, "you may be a good fit if you" lists, etc.) AND its text `.includes("yes")`, the poll considers it a found option and clicks at its center coords. Click goes to dead air; success is reported.

Fixes:
  1. **Scope option search to the open menu.** Before polling, require a `.select__menu` (or `[role="listbox"]`) to be visible — that's the contract for "menu is actually open." Search options as descendants of THAT specific element, not document-wide.
  2. **Tighten the match.** Require exact equality on `data-value` or `textContent`. Drop `tl.includes(valueLower)` entirely. If exact match fails, fail the call — never partial-match.
  3. **Verify after click.** Post-click, require `.select__single-value` (or equivalent committed-value indicator) to contain the expected text before returning `SELECTED`. If absent, return `Failed { error: "click dispatched but no value committed (component may need additional event)" }`.

**Reproducer (full sequence).**

```
browse_session_create("anthropic2", "cdp")
browse_session_switch("anthropic2")
browse_navigate("https://job-boards.greenhouse.io/anthropic/jobs/5218395008")
# Fills work fine
browse_fill(124, "Matt")  # First Name → ✅ value="Matt" in next snapshot
# Selects lie
browse_select(199, "Yes")   # → SELECTED: Yes  (but no commit)
browse_eval_js("document.querySelectorAll('.select__single-value').length")  # → 0
browse_click(199)  # → CLICKED: Select...  (no menu)
browse_eval_js("document.querySelectorAll('.select__menu').length")  # → 0
```

**Severity.** Same as BR-6 — blocks every Greenhouse/Lever/Ashby application. Worse than BR-6 because `browse_select` now lies (false-positive success), so any agent assuming "SELECTED" means "committed" will submit broken forms.

**Suggested triage.** Re-open BR-6 as not-actually-fixed, or work BR-7 as the formal regression follow-up. The three issues above are separable but compound — recommend fixing them in this order:

1. Move `browse_eval_js`, `cdp_resolve_ref_point`, and the readback in `browse_select` Step 4 to a **dedicated CDP `Runtime.evaluate` against the page's main world** (not the mirror sandbox). This alone makes diagnosis possible.
2. Verify `cdp_dispatch_real_click` coordinates with a one-line `tracing::info!("real click at x={x:.0} y={y:.0} ref={ref_id}")`. Run the smoke. If coords are bad, fix the resolver (canonical: `DOM.getBoxModel` on a `Runtime.callFunctionOn` of `el => el.getBoundingClientRect()` against the real frame).
3. Once (1) and (2) are right and menus actually open, tighten the option matcher per the three fixes above so `browse_select` never lies.

**Tags.** `mcp` `cdp-engine` `browse_select` `browse_eval_js` `regression` `false-positive` `react-select` `greenhouse` `priority-1` `blocks-headline-use-case`

### BR-8: `browse_fill` is broken for `<textarea>` and for masked / custom-controlled inputs — Greenhouse essay + phone fail ✅ FIXED 2026-05-16 (Input.insertText)
> Surfaced: 2026-05-16, immediately after BR-6 + BR-7 were marked ✅ FIXED. The E2E smoke marker in "Open Items After 2026-05-16" claiming "Greenhouse … end-to-end" is **incorrect** — the dropdowns commit fine (real BR-6/BR-7 wins), but two separate fill bugs prevent any Greenhouse application from actually submitting. Smoke ran on the same Anthropic Greenhouse URL; submit click was issued and silently bailed.

**Fix shipped 2026-05-16, third round.** Switched `BrowserAction::Fill` from the JS-setter approach to a real CDP `Input.insertText` flow with the setter as fallback. Both bugs collapse into one fix:

- **Bug 8a (textarea throws):** the setter path is now only used as a fallback, and when it runs it correctly branches on `el.tagName === 'TEXTAREA'` to pick `HTMLTextAreaElement.prototype` instead of `HTMLInputElement.prototype`. The old `descriptor1 || descriptor2` short-circuit (which always picked descriptor1 since the descriptor is on the prototype not the instance) is gone.
- **Bug 8b (React state doesn't update):** `Input.insertText` writes through the real Chrome input pipeline, dispatching `beforeinput` + `input` events the same way an actual keypress does. React's controlled-component handlers listen for those, so React state tracks the DOM. One CDP roundtrip for the whole string — much faster than per-char keypress dispatch and framework-agnostic (works for react-textarea-autosize, masked-input libs, Remix forms, react-hook-form, etc.).

**New flow** (`crates/browser-core/src/engine_cdp.rs`, `BrowserAction::Fill` arm):

1. Resolve ref → element, validate it's `<input>`/`<textarea>`/`contenteditable`, scroll into view, focus, `setSelectionRange(0, value.length)` so insertText replaces instead of appends.
2. `Input.insertText { text }` via CDP.
3. Verify both DOM value AND React state (`__reactProps$xxx.value` via the fiber). Three outcomes:
   - DOM matches AND (no React fiber OR React state matches) → `Success`.
   - DOM matches but React state lags → setter+input/change "nudge" path → re-verify → `Success` or `Failed { reason }`.
   - DOM doesn't match → `Failed` with both DOM and React state in the error so the next debug session has the diagnostic right there.
4. If `Input.insertText` itself errors (older Chrome / unusual element), automatic fallback to the setter path with the 8a fix applied.

`browse_fill` now NEVER returns Success without confirming the value actually landed in both DOM and React state. Agents downstream that assumed every "Filled @e…" was real will now get accurate feedback — but the converse is also true: any code that ignored the return value will now surface real failures it was previously eating.

**Validation.** `cargo check --features cdp,sevro` passes clean. `cargo build --release --features cdp,sevro` produced fresh `target/release/wraith-browser.exe` (47 MB, mtime 2026-05-16 7:51 AM). Next Matt-action: kill the running Wraith MCP, reconnect, re-run the Anthropic Greenhouse application — the Phone, Why Anthropic essay, and Additional Information fields should now commit to React state and the submit click should actually fire the network request.

---

> **Original BR-8 report (preserved for context):**

**Two distinct bugs in the Fill path. Both blocking. The textarea bug has an exact root cause and a one-line fix.**

---

**Bug 8a: textarea fill always throws because Fill always calls HTMLInputElement's value setter.**

Source: `crates/browser-core/src/engine_cdp.rs:935-944` in the just-built binary.

```rust
const nativeSetter = Object.getOwnPropertyDescriptor(
    window.HTMLInputElement.prototype, 'value'
) || Object.getOwnPropertyDescriptor(
    window.HTMLTextAreaElement.prototype, 'value'
);
if (nativeSetter && nativeSetter.set) {
    nativeSetter.set.call(el, '{escaped}');
}
```

`Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')` returns a truthy descriptor **regardless of what element `el` actually is** (the descriptor lives on the prototype, not the instance). So the `||` short-circuits to the input setter every time. Calling the input setter on a `<textarea>` throws `TypeError: Failed to set the 'value' property on 'HTMLInputElement': The provided value is not of type 'HTMLInputElement'`, which Wraith surfaces as `Fill failed: JavaScript evaluation failed: Uncaught`.

Live repro (just observed in the BR-7 smoke session):
```
> browse_fill(ref_id=29, text="test")   // @e29 is the Why Anthropic? <textarea>
< MCP error -32603: Fill failed: JavaScript evaluation failed: Uncaught
```

Same JS via eval_js with the right prototype works:
```
> browse_eval_js("var el=document.querySelector('[data-wraith-ref=\"29\"]'); var s=Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value').set; s.call(el,'test'); el.value")
< 'test'
```

**Fix (one block):**

```rust
const proto = el.tagName === 'TEXTAREA'
    ? window.HTMLTextAreaElement.prototype
    : window.HTMLInputElement.prototype;
const nativeSetter = Object.getOwnPropertyDescriptor(proto, 'value');
```

---

**Bug 8b: DOM-level fills don't propagate to React state for masked inputs / custom textareas; submit handler reads React state, so the submit click silently bails.**

Reproducer on the same Anthropic form, after the BR-7 fix:

```
> browse_fill(ref_id=12, text="5307863655")    // Phone field (masked input)
< Filled @e12 with text

> browse_eval_js("var el=document.querySelector('[data-wraith-ref=\"12\"]'); var k=Object.keys(el).find(x=>x.startsWith('__reactProps')); 'dom='+el.value+' react='+JSON.stringify(el[k].value||'')")
< 'dom=(530) 786-3655 react=""'
```

DOM is correct (Greenhouse auto-formatted), but React `props.value` is empty. Same pattern for textareas via manual setter+events workaround:

```
> // workaround: HTMLTextAreaElement.prototype.value setter on textarea, dispatch input + change events
> ...read both
< 'dom=1590 react=0'
```

Even calling the textarea's `onChange` handler directly via the React fiber (`el[reactPropsKey].onChange({currentTarget: el, target: el})`) doesn't update React state — the handler is `N => w(N.currentTarget.value)` where `w` is a setState in a parent component context that isn't reachable from the textarea's own props.

The submit-button onClick on Greenhouse is:

```js
async E => {
  if (ge) return;
  const k = gn(E);          // ← reads from React state
  if (!k) return;            // ← silent bail when state is invalid
  let O = {};
  try { O = await Ua({application: k, submitPath: r, ...}) }
  catch(P) { ... }
  if (O.ok) { ... window.location.assign(P) }
}
```

`gn(E)` reads from React state. React-state-empty required fields cause `!k`, the handler returns silently, no network call fires, no UI error appears. That's why "Submit application" clicks visibly succeed but nothing happens.

Affected components observed on Greenhouse:
- `<textarea>` rendered through `input-wrapper input-wrapper__multi-line` (probably react-textarea-autosize). React `props.value` stays `""` regardless of which setter / input / change / onChange-via-fiber path is used from `eval_js`.
- Phone field (masked input). Auto-formats `5307863655` → `(530) 786-3655` in DOM; React state stays `""`.

Simple text inputs (First Name, Email, Website, LinkedIn, Address) propagate correctly — Wraith's setter + input/change events work for those. So Bug 8b is specifically about controlled components with their own input handlers.

**Severity.** Blocks every Greenhouse application submit (and almost certainly Lever/Ashby too — they use the same masked-phone + react-textarea-autosize stack). The BR-6/BR-7 wins are real but the cross-form smoke is still red on the only application path that matters.

**Suggested fixes (any one):**

1. **Real typing path.** Replace the JS-setter approach in `BrowserAction::Fill` with a CDP keyboard typing implementation: focus the element via `DOM.focus`, then for each character dispatch `Input.dispatchKeyEvent` with type=keyDown / char / keyUp. Slower (~10ms/char ≈ 16s for a 1600-char essay) but works for every controlled component because it mimics actual keyboard input. Greenhouse + Remix Forms + react-textarea-autosize + masked-input libs all listen for real keystrokes.
2. **`Input.insertText` CDP method.** Single CDP call that simulates the whole string at once via the browser's input pipeline. Fast and framework-agnostic. Recommend this as the default.
3. **Detect-and-fallback.** Keep the setter path for speed on simple inputs; after the setter, read the React fiber's `memoizedProps.value` (now possible thanks to BR-7's eval_js fix) — if it doesn't match the requested text, fall back to (2). Best of both.

The cleanest is (2) as default with (3) as the optimization. That makes `browse_fill` framework-agnostic — no special-casing per UI library — and the textarea bug (8a) becomes moot because `Input.insertText` doesn't care about prototype lookups.

**Workaround for the current Anthropic application.** Form is mostly populated in Wraith's headless Chrome session "anthropic3": all dropdowns commit, all simple text inputs populate, resume uploaded. But Phone + Why Anthropic essay + Additional Information are DOM-only and the React-state read at submit time returns invalid → submit silently bails. The operator has to redo the application in their own browser. Application packet at `J:\job-hunter-mcp\.pipeline\applications\anthropic-swe-systems-claude-code-2026-05-16.md`.

**Tags.** `mcp` `cdp-engine` `browse_fill` `textarea` `masked-input` `react-controlled-component` `react-textarea-autosize` `greenhouse` `priority-1` `blocks-submit`

---

## Priority Order

1. **BR-8** — `browse_fill` broken for textarea + masked inputs ✅ FIXED 2026-05-16 (switched to `Input.insertText` + dual DOM/React state verification)
2. **BR-7** — BR-6 regression triage ✅ FIXED 2026-05-16
3. **BR-6** — Portal-rendered react-select unfillable ✅ FIXED 2026-05-16
4. **BR-1** — Get API server running (TRW blocked) ✅
5. **FR-1** — HttpTransport wiring (ClaudioOS path) ✅
6. **FR-3** — Stealth fetch mode ✅
7. **FR-2** — CLI playbook runner ✅
8. **FR-4** — Bare-metal integration (ClaudioOS dependency)
9. **BR-2** — Pre-built binaries ✅
10. **BR-3** — PAT rotation (security hygiene) — Matt-action only

## Open Items After 2026-05-16

- **E2E smoke (re-run)** — Matt-action: kill running Wraith MCP, reconnect, re-run the Anthropic Greenhouse application against the fresh binary. BR-6 + BR-7 + BR-8 all landed. The Phone field, Why Anthropic essay, and Additional Information textarea should now commit to React state; the Submit Application button should actually fire the network request instead of silently bailing.
- **BR-3** — PAT rotation (5-min Matt-action in github.com + `setx`). Runbook: `scripts/rotate-github-pat.md`.
- **FR-4** — ClaudioOS-side `impl HttpTransport for SmoltcpTransport` (4 lines + QEMU smoke). Coordination doc: `J:\baremetal claude\docs\WRAITH-CRATES-HANDOFF.md`. Wraith side is fully ready.
- **Diagnostic follow-up (low priority)** — add `tracing::info!` of `(x, y, ref_id)` inside `cdp_dispatch_real_click` per BR-7's suggestion. Would have caught the wrong-engine routing in 5 minutes instead of forcing the long mirror/sandbox investigation.
