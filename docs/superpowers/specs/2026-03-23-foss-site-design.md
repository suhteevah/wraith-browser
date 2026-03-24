# OpenClaw FOSS Site — Design Spec

**Date:** 2026-03-23
**Status:** Approved
**Author:** Matt + Claude

---

## 1. Overview

A documentation, homepage, and playground site for the open-source OpenClaw Browser (Wraith) project. Fully isolated from all enterprise code and artifacts. Deployed to Vercel via CLI.

### Goals
- Provide comprehensive docs for the FOSS browser engine and its 140+ MCP tools (exact count derived from source at build time)
- Give developers a fast path from discovery → install → first scrape
- Interactive playground tutorials that showcase capabilities without requiring a backend
- Blog/changelog for release communication
- Community hub (Discord primary, Matrix bridged later)

### Non-Goals
- Enterprise features, pricing, sales funnels
- Live sandbox with hosted Wraith backend (phase 2, not this spec)
- User accounts or authentication

---

## 2. White-Room Rules

Hard boundaries between FOSS and enterprise. These are non-negotiable.

### Excluded from FOSS site (enterprise-only)
- `crates/api-server/` — multi-tenant REST API
- `dashboard/` — enterprise admin UI
- JWT authentication, refresh tokens
- Teams, organizations, RBAC
- Billing, metering, Stripe integration
- SSO (OIDC, SAML)
- Swarm orchestration (multi-session fan-out)
- WebSocket streaming API
- Admin routes
- Fly.io managed hosting config
- Multi-region replication
- Enterprise pricing tiers
- Sales CTAs, `sales@wraith.dev`

### Included in FOSS site (open-source crates)
- `browser-core` — native rendering engine
- `mcp-server` — 140+ MCP tools (count derived from source at build time)
- `cli` — local CLI interface
- `content-extract` — article parsing, markdown conversion
- `cache` — SQLite page cache, embeddings, knowledge graph
- `search-engine` — Tantivy full-text + web search
- `agent-loop` — MCTS planning, agent decision loop
- `scripting` — Rhai automation scripts
- `identity` — credential vault, TOTP, encryption
- `sevro/headless` — native rendering engine (Servo-derived)

### The line
Single-user local tool = FOSS. Multi-tenant hosted platform = enterprise.

### Enterprise reference
One tasteful outbound link allowed: "Need managed hosting & team features?" pointing to the enterprise site. No sales funnel, no pricing table, no feature-gating language.

---

## 3. Technical Architecture

### Framework
- **Geistdocs** (Next.js 16 + Fumadocs)
- MDX authoring with auto-routing from `content/docs/`
- Built-in AI chat ("Ask AI") trained on docs content
- Built-in search

### Location
- `foss-site/` at repository root
- Completely separate from `website/` (enterprise marketing) and `dashboard/` (enterprise UI)

### Deployment
- **Target:** Vercel
- **Method:** `vercel deploy` from CLI (GitHub Actions not available)
- **Domain:** TBD (e.g., `openclaw.dev` or `wraith.dev`)
- **Output:** SSG where possible, SSR only if needed for AI chat

### Directory Structure
```
foss-site/
├── app/                              # Next.js app directory
│   ├── layout.tsx                    # Root layout, Geist fonts
│   ├── page.tsx                      # Homepage
│   ├── community/page.tsx            # Community links
│   ├── playground/page.tsx           # Playground hub
│   ├── blog/page.tsx                 # Blog listing
│   ├── blog/[slug]/page.tsx          # Individual blog posts
│   └── not-found.tsx                 # Custom 404 with enterprise redirect
├── components/
│   ├── playground-replay.tsx         # Interactive terminal replay
│   ├── install-block.tsx             # Copy-paste install commands
│   └── terminal-demo.tsx             # Homepage embedded demo
├── content/
│   ├── docs/
│   │   ├── meta.json                 # Sidebar ordering for top-level sections
│   │   ├── getting-started/
│   │   │   ├── installation.mdx
│   │   │   ├── first-session.mdx
│   │   │   └── hello-world-scrape.mdx
│   │   ├── mcp-tools/
│   │   │   ├── index.mdx            # Overview + categories
│   │   │   ├── navigation.mdx       # browse_navigate, browse_back, etc.
│   │   │   ├── dom.mdx             # dom_query_selector, dom_focus, etc.
│   │   │   ├── cookies.mdx         # cookie_get, cookie_set, etc.
│   │   │   ├── interaction.mdx      # browse_click, browse_fill, etc.
│   │   │   ├── extraction.mdx       # extract_markdown, extract_article, etc.
│   │   │   ├── cache.mdx            # cache_get, cache_search, etc.
│   │   │   ├── identity.mdx         # browse_vault_store, browse_vault_get, etc.
│   │   │   ├── session.mdx          # browse_session_create, browse_session_list, etc.
│   │   │   ├── search.mdx           # browse_search, embedding_search, etc.
│   │   │   ├── entities.mdx         # entity_add, entity_query, entity_relate, etc.
│   │   │   ├── automation.mdx       # script_run, workflow_*, dag_*, etc.
│   │   │   ├── time-travel.mdx      # timetravel_*, page_diff
│   │   │   ├── plugins.mdx         # plugin_register, plugin_execute, etc.
│   │   │   ├── telemetry.mdx       # telemetry_metrics, telemetry_spans
│   │   │   └── advanced.mdx        # stealth, TLS, DNS, MCTS, etc.
│   │   ├── guides/
│   │   │   ├── web-scraping.mdx
│   │   │   ├── form-filling.mdx
│   │   │   ├── credential-vault.mdx
│   │   │   ├── knowledge-graph.mdx
│   │   │   └── automation-scripts.mdx
│   │   ├── architecture/
│   │   │   ├── engine-overview.mdx   # Native engine, no Chrome
│   │   │   ├── snapshot-model.mdx    # @ref IDs, DOM snapshots
│   │   │   └── mcp-protocol.mdx     # How the MCP server works
│   │   ├── knowledge-graph/
│   │   │   ├── page-cache.mdx       # SQLite cache, raw HTML storage
│   │   │   ├── embeddings.mdx       # Vector search, upsert, similarity
│   │   │   ├── entity-resolution.mdx # Entity linking, knowledge graph
│   │   │   └── full-text-search.mdx  # Tantivy index
│   │   ├── self-hosting/
│   │   │   ├── docker.mdx
│   │   │   └── configuration.mdx
│   │   └── cli-reference/
│   │       ├── commands.mdx
│   │       └── transport-modes.mdx
│   ├── blog/
│   │   └── introducing-openclaw.mdx  # Launch announcement
│   └── playground/
│       ├── first-scrape.json         # Session recording
│       ├── fill-a-form.json
│       ├── knowledge-graph.json
│       └── vault-and-login.json
├── scripts/
│   └── generate-tool-docs.ts        # Pre-build: parse MCP tools → MDX + tool-count.json
├── lib/
│   └── replay-parser.ts             # Parse session recordings
├── public/
│   ├── og-image.png
│   └── favicon.ico
├── geistdocs.tsx                     # Geistdocs config
├── next.config.ts
├── package.json
├── tailwind.config.ts
└── tsconfig.json
```

---

## 4. Homepage Design

Adapted from the existing `website/page.tsx` content, enterprise references stripped.

### Sections (top to bottom)
1. **Hero** — "Run 7,000 browser sessions on a single machine"
   - Subtext: "A native browser engine for AI agents. No Chrome. No Selenium. ~50ms per page."
   - Primary CTA: Install command block (`cargo install openclaw-browser`)
   - Secondary CTA: "Read the docs"

2. **Terminal demo** — Embedded `<PlaygroundReplay />` showing the "first scrape" tutorial auto-playing

3. **Feature cards (3):**
   - Native Engine — 15MB binary, no Chrome dependency, ~50ms per page
   - 140+ MCP Tools — navigation, extraction, vault, knowledge graph, automation
   - Searchable Knowledge Graph — every page cached, embedded, entity-linked, full-text indexed

4. **Competitor comparison table** — Wraith vs Browserbase, Browserless, Apify. Adapted from existing but fully rewritten for FOSS context. Remove: Cost/100K pages, SOC 2 Type II, Data residency, ATS integrations (all enterprise). Keep/add: engine size, concurrency, MCP tool count, credential vault (local), knowledge graph (local), anti-bot capabilities.

5. **How it works** — 3-step sequence rewritten for local/MCP usage (NOT the enterprise API flow):
   1. Install the binary (`cargo install` / Docker / download)
   2. Connect via MCP (`openclaw-browser serve --transport stdio`) or use the CLI
   3. Automate: navigate, extract, build knowledge graphs

6. **Install methods:**
   - Cargo: `cargo install openclaw-browser`
   - Docker: `docker pull openclaw/browser`
   - Binary: download links per platform

7. **Footer:**
   - License: AGPL-3.0
   - Community: Discord, Matrix (placeholder)
   - Docs, GitHub, Blog
   - "Need managed hosting & team features?" → enterprise link

---

## 5. Playground — Interactive Tutorials

### Component: `<PlaygroundReplay />`
A React component that renders a simulated terminal session.

**Props:**
```typescript
interface PlaygroundReplayProps {
  recording: SessionRecording;
  autoPlay?: boolean;
  speed?: number; // playback speed multiplier
}

interface SessionRecording {
  title: string;
  description: string;
  steps: SessionStep[];
}

interface SessionStep {
  type: 'command' | 'output' | 'annotation';
  content: string;        // MCP tool call JSON or response
  delay_ms: number;       // timing for auto-play
  annotation?: string;    // explanatory text shown alongside
}
```

**Behavior:**
- Steps render one at a time
- "Next" button or auto-play mode
- Commands get syntax highlighting (JSON)
- Outputs render with appropriate formatting (markdown preview for extract, table for snapshots)
- Annotations appear as callout boxes explaining what's happening

### Initial tutorials (4)
1. **"Your first scrape"** — `browse_navigate` → `browse_snapshot` → `extract_markdown` (3 steps)
2. **"Fill a form"** — navigate → snapshot → `browse_fill` by @ref → `browse_submit_form` (5 steps)
3. **"Build a knowledge graph"** — scrape 5 pages → `cache_search` → `entity_query` → `entity_visualize` (8 steps)
4. **"Vault & login"** — `browse_vault_store` → `browse_navigate` → `browse_login` → verify (6 steps)

### Recording format
JSON files in `content/playground/`. Captured from real Wraith MCP sessions, then annotated by hand.

---

## 6. MCP Tools Reference — Auto-Generation

The MCP tool count (140+) should be derived from source at build time, never hardcoded.

### Strategy
- **Pre-build script** (`scripts/generate-tool-docs.ts`): a Node.js script that runs `openclaw-browser serve --transport stdio --list-tools` (or parses `make_tool()` calls from `crates/mcp-server/src/server.rs`) to extract tool names, descriptions, and parameter schemas
- Output: one MDX file per category in `content/docs/mcp-tools/`, auto-generated with a `<!-- AUTO-GENERATED -->` header
- Hand-written category intros and usage notes in separate MDX files that import/wrap the generated content
- The script also emits a `tool-count.json` used by the homepage and AI chat prompt so the count is always accurate
- **Conditional tools:** Tools gated behind `#[cfg(feature = "cdp")]` are documented with a note that they require the `cdp` feature flag at build time

### Categories (15 categories, mapped from tool prefixes)
| Category | MDX file | Prefix patterns | Example tools |
|----------|----------|-----------------|---------------|
| Navigation | `navigation.mdx` | `browse_navigate`, `browse_back`, `browse_forward`, `browse_reload` | 6 tools |
| Interaction | `interaction.mdx` | `browse_click`, `browse_fill`, `browse_type`, `browse_hover`, `browse_select`, `browse_key_press`, `browse_scroll*`, `browse_upload_file`, `browse_submit_form`, `browse_dismiss_overlay`, `browse_custom_dropdown` | 14 tools |
| Extraction | `extraction.mdx` | `extract_*`, `browse_extract`, `browse_snapshot` | 7 tools |
| Cache | `cache.mdx` | `cache_*` | 10 tools |
| Identity/Vault | `identity.mdx` | `browse_vault_*`, `browse_login`, `vault_*`, `identity_*`, `fingerprint_*` | 15 tools |
| Session | `session.mdx` | `browse_session_*`, `browse_tabs`, `browse_config`, `browse_engine_status` | 8 tools |
| Search | `search.mdx` | `browse_search`, `embedding_*` | 5 tools |
| Entities | `entities.mdx` | `entity_*` | 8 tools |
| DOM | `dom.mdx` | `dom_*`, `browse_enter_iframe` | 5 tools |
| Cookies | `cookies.mdx` | `cookie_*` | 5 tools |
| Automation | `automation.mdx` | `script_*`, `workflow_*`, `dag_*`, `swarm_fan_out`, `swarm_collect`, `swarm_run_playbook`, `swarm_list_playbooks`, `swarm_playbook_status`, `swarm_dedup_*`, `swarm_verify_submission` | 20 tools |
| Time Travel | `time-travel.mdx` | `timetravel_*`, `page_diff` | 6 tools |
| Plugins | `plugins.mdx` | `plugin_*` | 4 tools |
| Telemetry | `telemetry.mdx` | `telemetry_*` | 2 tools |
| Advanced | `advanced.mdx` | `stealth_*`, `tls_*`, `dns_*`, `network_*`, `auth_detect`, `site_fingerprint`, `browse_solve_captcha`, `prefetch_predict`, `mcts_*` | 12 tools |

### Swarm tools — FOSS vs Enterprise boundary
All `swarm_*` tools registered in the MCP server are **FOSS** — they are local parallelism tools that run on the user's machine. The enterprise "swarm orchestration" in `api-server/` is a separate multi-tenant coordination layer that manages swarm jobs across remote sessions via REST API. The docs must clarify this:
- FOSS: `swarm_fan_out`, `swarm_collect`, `swarm_run_playbook`, `swarm_list_playbooks`, `swarm_playbook_status`, `swarm_dedup_*`, `swarm_verify_submission`
- Enterprise-only: the `/api/v1/swarm` REST endpoints in `api-server/`

### ATS playbooks
The MCP server ships with built-in playbooks (`greenhouse-apply`, `ashby-apply`, `lever-apply`). These are local automation scripts and are FOSS. The enterprise "Native ATS Integration" refers to managed API connectors with webhook callbacks — a different thing. The FOSS docs should document playbooks as "automation templates" without enterprise integration language.

---

## 7. Blog & Changelog

### Format
MDX files in `content/blog/`. Blog routing is handled via a custom `app/blog/` route (not the Fumadocs docs source). Fumadocs routes `content/docs/` automatically, but blog content needs its own `app/blog/page.tsx` (listing) and `app/blog/[slug]/page.tsx` (individual posts) that load MDX from `content/blog/` using `next-mdx-remote` or Fumadocs' `createMDXSource` with a separate content source config.

### Launch content
- **"Introducing OpenClaw Browser"** — what it is, why we built it, how to get started
- First entry in a changelog series for version releases

### Ongoing
- Release notes per version
- Technical deep-dives (engine architecture, MCP tool design, etc.)
- Community spotlights ("Built with Wraith")

---

## 8. Community Page

### Content
- **Discord** — primary community (invite link, live at launch)
- **Matrix** — placeholder at launch, bridged to Discord later
- **Contributing guide** — link to `CONTRIBUTING.md` in repo
- **Code of conduct** — link to `CODE_OF_CONDUCT.md`
- **"Built with Wraith" showcase** — empty at launch, community-submitted projects over time
- **GitHub** — repo link, issues, discussions

---

## 9. AI Chat Configuration

Geistdocs built-in "Ask AI" feature.

### Prompt
Configured in `geistdocs.tsx`. The tool count is injected from `tool-count.json` at build time:
```
You are the OpenClaw Browser documentation assistant. Help developers use
Wraith — a native, AI-agent-first browser with {toolCount} MCP tools. Answer
questions about installation, MCP tool usage, the knowledge graph, vault,
scripting, and self-hosting. You only know about the open-source version.
Do not reference enterprise features, pricing, or managed hosting.
```

### Context
AI chat is trained on all MDX content in `content/docs/`. No enterprise content exists in the FOSS site, so the white-room is enforced by construction.

---

## 10. License

The root `LICENSE` file is AGPL-3.0. The existing `website/page.tsx` references MPL-2.0 in two places (badge and footer) — this is incorrect and must be fixed. The FOSS site uses AGPL-3.0 consistently. The Sevro rendering engine has its own license at `sevro/LICENSE` which should be referenced in the architecture docs.

---

## 11. SEO & LLM Discoverability

- **Open Graph:** Custom `og-image.png` for social sharing, page-level metadata via MDX frontmatter
- **Structured data:** JSON-LD for SoftwareApplication on the homepage
- **`llms.txt`:** Geistdocs provides `/llms.txt` (all docs as plain markdown) and `.md` URL extension — explicitly enable and test both. These are high-value for an AI-agent-first tool whose users will feed docs into LLMs.
- **404 page:** Custom `app/not-found.tsx` — "Looking for enterprise features? Visit [enterprise link]" as a clean redirect for old enterprise URLs
- **Sitemap:** Auto-generated by Geistdocs

---

## 12. Fumadocs Sidebar Configuration

Each directory under `content/docs/` needs a `meta.json` file for sidebar ordering. Without these, sidebar order is alphabetical. Example:

```json
// content/docs/meta.json
{
  "pages": [
    "getting-started",
    "mcp-tools",
    "guides",
    "architecture",
    "knowledge-graph",
    "self-hosting",
    "cli-reference"
  ]
}
```

Each subdirectory also needs its own `meta.json` for page ordering within that section.

---

## 13. Deployment

### Platform
Vercel, deployed via CLI.

### Setup
```bash
cd foss-site
pnpm install       # Geistdocs uses pnpm by default
vercel link        # connect to Vercel project
vercel deploy      # preview
vercel --prod      # production
```

### CI alternative
Since GitHub Actions are not available, deployments are manual via `vercel deploy` from local machine or CNC. A cron-based auto-deploy from CNC could be set up later if needed.

### Environment variables
Minimal — only what Geistdocs AI chat needs (AI Gateway OIDC if using Vercel AI features).

---

## 14. Phases

### Phase 1 (this spec)
- Geistdocs scaffold with all doc sections
- Homepage with install block and terminal demo
- 4 playground tutorials (pre-recorded)
- MCP tools reference (auto-generated from source)
- Blog with launch post
- Community page (Discord live, Matrix placeholder)
- Deploy to Vercel

### Phase 2 (follow-up)
- Live sandbox — hosted Wraith endpoint for anonymous sessions
- Matrix bridge to Discord
- "Built with Wraith" showcase with submissions
- Versioned docs (per release)
- Internationalization (if demand warrants)

---

## 15. Open Questions

1. **Domain:** `openclaw.dev`, `wraith.dev`, or something else? — TBD
2. **Discord server:** Needs to be created before launch
3. **Auto-generation pipeline:** Pre-build script to parse MCP tools from Rust source — either invoke the binary with `--list-tools` or parse `make_tool()` calls from `server.rs`. The binary approach is cleaner but requires the binary to be available at build time on Vercel (could use a pre-built tools manifest committed to the repo instead).
4. **Session recordings:** Need to capture 4 real MCP sessions for playground content
5. **License confirmation:** Verify AGPL-3.0 is the intended license for the FOSS site — the existing `website/` references MPL-2.0 which may have been intentional for the marketing site but conflicts with the root LICENSE
