# Wraith Browser

**The AI-agent-first web browser -- built in Rust, designed for LLM control.**

[![License: AGPL-3.0](https://img.shields.io/badge/License-AGPL--3.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.75%2B-orange.svg)](https://www.rust-lang.org/)
[![Tests](https://img.shields.io/badge/tests-348%20passing-brightgreen.svg)]()
[![MCP Tools](https://img.shields.io/badge/MCP%20tools-105-blue.svg)]()

---

Wraith is a native Rust browser engine purpose-built for AI agents. No Chrome dependency. No Node.js. Ships as a single ~15MB binary or MCP server with 105 tools. Handles protected sites that break traditional automation frameworks.

## Why Wraith

Every AI browser automation tool today wraps Playwright or Puppeteer -- JavaScript runtimes controlling a 300MB Chrome process. Wraith takes a different approach:

| | Wraith | Playwright/Puppeteer |
|---|---|---|
| Chrome required | No | Yes (300MB+) |
| Memory per session | 5-50 MB | 300-500 MB |
| Page fetch (static) | ~50ms | 1-3 seconds |
| Binary size | ~15 MB | ~300 MB + runtime |
| Startup time | <100ms | 2-5 seconds |
| Concurrent sessions (16GB) | 50-100+ | 6-8 |
| Protected site handling | Multi-tier adaptive | Limited |
| MCP native | Yes (105 tools) | No |

## Quick Start

### Build

```bash
git clone https://github.com/suhteevah/wraith-browser.git
cd wraith-browser
cargo build --release
```

### Connect to Claude Code

```bash
claude mcp add wraith ./target/release/openclaw-browser -- serve --transport stdio
```

Your AI agent immediately gains 105 browser tools.

### CLI

```bash
# Navigate and see interactive elements
wraith-browser navigate https://example.com

# Extract content as clean markdown
wraith-browser extract https://docs.rust-lang.org --max-tokens 4000

# Search the web
wraith-browser search "rust async runtime benchmarks"

# Autonomous browsing task
ANTHROPIC_API_KEY=sk-... wraith-browser task "Find remote Rust jobs on HN"

# Manage encrypted credentials
wraith-browser vault store --domain github.com --kind password --identity user@example.com
```

## Architecture

```
                     AI Agent (Claude Code, Cursor, custom)
                                    |
                              MCP Protocol (stdio)
                                    |
                    +---------------v----------------+
                    |       MCP Server (105 tools)   |
                    +---------------+----------------+
                                    |
                    +---------------v----------------+
                    |     BrowserEngine Trait         |
                    |  SevroEngine  |  NativeEngine  |
                    +------+--------+-------+--------+
                           |                |
          +----------------v--+    +--------v---------+
          | Sevro Headless     |    | Pure HTTP Client  |
          | - QuickJS (JS)     |    | - HTTP/1.1 + 2   |
          | - DOM Bridge       |    | - HTML5 parser    |
          | - Adaptive access  |    | - ~50ms/page      |
          +--------------------+    +-------------------+
```

### 10 Crates

| Crate | Purpose |
|-------|---------|
| `browser-core` | Unified engine trait, network layer, vision, swarm, plugins |
| `sevro-headless` | Headless engine -- HTTP, DOM parsing, QuickJS, adaptive site access |
| `agent-loop` | LLM agent cycle -- MCTS planning, time-travel, workflows, task DAGs |
| `cache` | SQLite knowledge store, embeddings, entity graph, semantic diffing |
| `content-extract` | Readability extraction, markdown conversion, OCR, PDF |
| `identity` | Encrypted credential vault, browser profiles, auth flows |
| `mcp-server` | MCP protocol server (105 tools, stdio transport) |
| `search-engine` | DuckDuckGo, SearXNG metasearch, local Tantivy index |
| `scripting` | Rhai sandboxed scripting engine (userscripts) |
| `cli` | Binary with subcommands |

## Site Compatibility

Wraith uses an adaptive multi-tier approach to access protected sites. When standard requests are blocked, the engine automatically escalates through progressively more sophisticated access methods. Sites that block every Playwright and Puppeteer script are accessible through Wraith's layered architecture.

Verified against major e-commerce, job search, review, and enterprise platforms.

## MCP Tools (105)

Every capability has a native MCP tool. AI agents have full admin control with zero CLI interaction.

| Category | Count | Tools |
|----------|-------|-------|
| Navigation | 7 | navigate, back, forward, reload, scroll, wait, wait_navigation |
| Interaction | 7 | click, fill, select, type, hover, key_press, dom_focus |
| DOM | 3 | query_selector, get_attribute, set_attribute |
| Extraction | 9 | extract, snapshot, eval_js, screenshot, pdf, article, markdown, plain_text, ocr |
| Search | 1 | search |
| Vault | 12 | store, get, list, delete, totp, rotate, audit, lock, unlock, approve_domain, revoke_domain, check_approval |
| Cookies | 4 | get, set, save, load |
| Cache | 10 | search, get, stats, purge, pin, tag, domain_profile, find_similar, evict, raw_html |
| Intelligence | 6 | entity_query, network_discover, site_fingerprint, page_diff, auth_detect, dns_resolve |
| Entity Graph | 6 | add, relate, merge, find_related, search, visualize |
| Identity | 4 | fingerprint_list, fingerprint_import, identity_profile, tls_profiles |
| Stealth | 1 | stealth_status |
| Plugins | 4 | register, execute, list, remove |
| Telemetry | 2 | metrics, spans |
| Agent | 1 | browse_task |
| Workflow | 4 | start_recording, stop_recording, replay, list |
| Time-Travel | 5 | summary, branch, replay, diff, export |
| Task DAG | 7 | create, add_task, add_dependency, ready, complete, progress, visualize |
| MCTS Planning | 2 | plan, stats |
| Prefetch | 1 | predict |
| Swarm | 2 | fan_out, collect |
| Embeddings | 2 | search, upsert |
| Config | 1 | browse_config |

## DOM Snapshots

Instead of dumping raw HTML, Wraith produces compact agent-readable snapshots:

```
[Page: Search Results — https://example.com/search?q=engineer]

@e1  [search]  "" placeholder="Search"
@e2  [search]  "" placeholder="Location"
@e3  [button]  "Find Results"
@e4  [link]    "Senior Engineer — Company A" -> /view?id=123
@e5  [link]    "Backend Engineer — Company B" -> /view?id=456
@e6  [link]    "Next >"

[6 interactive elements | ~40 tokens]
```

An agent reads this and responds: `ACTION: click @e4`

## Agent Intelligence

- **MCTS Action Planning** -- Monte Carlo Tree Search over action sequences
- **Predictive Pre-Fetching** -- anticipates next URLs from task context
- **Time-Travel Debugging** -- branch, replay, and diff agent decision paths
- **Workflow Recording** -- capture flows, parameterize, replay
- **Task DAGs** -- parallel subtasks with dependency resolution
- **Knowledge Graph** -- cross-site entity resolution

## Credential Security

- AES-256-GCM encryption at rest with Argon2id key derivation
- Credentials never appear in LLM context windows or log files
- Per-domain access controls
- Automatic TOTP 2FA generation
- Full audit trail of every credential access
- Secrets zeroized from memory immediately after use

## Intelligent Caching

Every page visited is cached, indexed, and searchable. Subsequent requests for the same content are served from the local knowledge store at microsecond latency. Cache TTLs adapt automatically per domain based on observed content change frequency.

- SQLite + Tantivy full-text search
- Semantic page diffing (detects meaningful changes between visits)
- Cross-site entity resolution
- Embedding store with cosine similarity search

## Plugin System

- **WASM plugins** (wasmtime) -- sandboxed, hot-reloadable, domain-specific extractors
- **Rhai scripting** -- userscripts that trigger on navigation events
- **Vision ML pipeline** (ort/ONNX) -- UI element detection for canvas/non-DOM content

## License

**AGPL-3.0** -- free and open source.

Use freely for personal projects, open source, research, and internal tools. If you modify Wraith and deploy it as a network service, modifications must be released under the same license.

### Commercial License

Companies that want to embed Wraith in proprietary products without open-source obligations can obtain a commercial license. Contact [ridgecellrepair@gmail.com](mailto:ridgecellrepair@gmail.com).

### Wraith Enterprise (Coming Q3 2026)

Managed browser automation as a service:
- Auto-scaling browser fleet
- Team credential vault with RBAC
- Centralized knowledge store
- Compliance dashboard and audit exports
- Dedicated support with SLA

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines. Key areas:

- Search provider integrations
- Auth flow detection patterns
- Documentation and examples

## Acknowledgments

Built with [scraper](https://github.com/causal-agent/scraper), [rquickjs](https://crates.io/crates/rquickjs), [lol_html](https://github.com/nickel-ob/lol-html), [Tantivy](https://github.com/quickwit-oss/tantivy), [rmcp](https://crates.io/crates/rmcp), [ort](https://crates.io/crates/ort), [wasmtime](https://crates.io/crates/wasmtime), and [petgraph](https://crates.io/crates/petgraph).

---

**Wraith** -- *the browser your AI agent deserves.*

Copyright (c) 2026 Matt Gates / Ridge Cell Repair LLC
