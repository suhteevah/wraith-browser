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

### BR-1: Enterprise API server status unknown
The API server (`crates/api-server`, Axum on :8080) may not be running anywhere. TRW needs it. Verify it's deployed and accessible from Vercel's serverless functions. Set `WRAITH_API_URL` env var in TRW's Vercel project.

### BR-2: Pre-built binaries don't exist
No GitHub Actions (banned). Need a local cross-compilation script. This blocks anyone else from using Wraith. Low priority unless shipping to customers.

### BR-3: Exposed GitHub PAT
PAT was exposed in a conversation transcript (noted in HANDOFF.md TODO #4). Needs rotation.

---

## Feature Requests

### FR-1: Wire HttpTransport into sevro-headless (HANDOFF TODO #5)
Replace direct reqwest calls in sevro-headless with the `HttpTransport` trait. This enables the no_std path for ClaudioOS bare-metal. Trait exists in `crates/transport/`, just not wired in yet.

### FR-2: `wraith run` CLI subcommand (HANDOFF TODO #7)
Load YAML playbooks from `playbooks/`, validate variables, dispatch steps. Playbook parser already implemented, MCP tool exists — just needs a CLI entrypoint. Real use case: `wraith run sofascore-tennis`.

### FR-3: HTTP-only stealth mode (HANDOFF TODO #8)
`wraith fetch <url>` with TLS fingerprinting for JSON APIs (no DOM needed). Use case: Sofascore, ESPN. Could be a library function: `use wraith_browser_core::stealth_fetch`. Fast path that skips Servo entirely.

### FR-4: Bare-metal integration testing (HANDOFF TODO #6)
Compile-verify `wraith-dom`, `wraith-transport`, `wraith-render` in ClaudioOS repo. Wire into kernel. These crates are in `J:\baremetal claude\crates\`.

---

## Priority Order

1. **BR-1** — Get API server running (TRW blocked)
2. **FR-1** — HttpTransport wiring (ClaudioOS path)
3. **FR-3** — Stealth fetch mode (high value, relatively small)
4. **FR-2** — CLI playbook runner (quality of life)
5. **FR-4** — Bare-metal integration (ClaudioOS dependency)
6. **BR-2** — Pre-built binaries (nice to have)
7. **BR-3** — PAT rotation (security hygiene)
