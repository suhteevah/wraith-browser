# Wraith Browser

**The AI-agent-first web browser -- built in Rust, designed for LLM control.**

[![License: AGPL-3.0](https://img.shields.io/badge/License-AGPL--3.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.75%2B-orange.svg)](https://www.rust-lang.org/)
[![Tests](https://img.shields.io/badge/tests-348%20passing-brightgreen.svg)]()
[![MCP Tools](https://img.shields.io/badge/MCP%20tools-109-blue.svg)]()

---

Wraith is a native Rust browser engine purpose-built for AI agents. No Chrome dependency. No Node.js. Ships as a single ~15MB binary or MCP server with 109 tools -- every capability accessible via MCP calls. Handles protected sites that break traditional automation frameworks.

## Why Wraith

| | Wraith | Playwright/Puppeteer |
|---|---|---|
| Chrome required | No | Yes (300MB+) |
| Memory per session | 5-50 MB | 300-500 MB |
| Page fetch (static) | ~50ms | 1-3 seconds |
| Binary size | ~15 MB | ~300 MB + runtime |
| Startup time | <100ms | 2-5 seconds |
| Concurrent sessions (16GB) | 50-100+ | 6-8 |
| Protected site handling | Multi-tier adaptive | Limited |
| MCP native | Yes (109 tools) | No |
| File upload | Yes (DataTransfer API) | Yes (setInputFiles) |
| Form submission | Yes (React/XHR-aware) | Yes |
| Custom dropdowns | Yes (combobox handler) | Manual scripting |
| Cookie import from Chrome | Yes (reads Chrome DB) | Manual |

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

Your AI agent immediately gains 109 browser tools -- full admin control with zero CLI interaction.

### CLI

```bash
# Navigate and see interactive elements
openclaw-browser navigate https://example.com

# Extract content as clean markdown
openclaw-browser extract https://docs.rust-lang.org --max-tokens 4000

# Search the web (supports OR queries)
openclaw-browser search "site:greenhouse.io QA engineer OR SDET remote"

# Autonomous browsing task
ANTHROPIC_API_KEY=sk-... openclaw-browser task "Find remote Rust jobs on HN"

# Manage encrypted credentials
openclaw-browser vault store --domain github.com --kind password --identity user@example.com
```

### Environment Variables (MCP mode)

| Variable | Purpose |
|----------|---------|
| `WRAITH_FLARESOLVERR` | URL for challenge solver service |
| `WRAITH_PROXY` | Primary HTTP/SOCKS5 proxy |
| `WRAITH_FALLBACK_PROXY` | Fallback proxy for access issues |
| `ANTHROPIC_API_KEY` | For autonomous browsing tasks |
| `BRAVE_SEARCH_API_KEY` | Optional search provider |

## Architecture

```
                     AI Agent (Claude Code, Cursor, custom)
                                    |
                              MCP Protocol (stdio)
                                    |
                    +---------------v----------------+
                    |       MCP Server (109 tools)   |
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
| `content-extract` | Readability extraction, markdown, OCR, PDF |
| `identity` | Encrypted credential vault, browser profiles, auth flows |
| `mcp-server` | MCP protocol server (109 tools, stdio transport) |
| `search-engine` | DuckDuckGo, SearXNG metasearch, OR query splitting, local Tantivy index |
| `scripting` | Rhai sandboxed scripting engine (userscripts) |
| `cli` | Binary with subcommands |

## Site Compatibility

Wraith uses an adaptive multi-tier approach to access protected sites. When standard requests are blocked, the engine automatically escalates through progressively more sophisticated access methods.

Verified against major e-commerce, job search, review, and enterprise platforms including sites that block all Playwright and Puppeteer scripts.

## MCP Tools (109)

Every capability has a native MCP tool. AI agents have full admin control with zero CLI interaction.

| Category | Count | Highlights |
|----------|-------|------------|
| Navigation | 7 | navigate, back, forward, reload, scroll, wait, wait_navigation |
| Interaction | 7 | click, fill, select, type (realistic delays), hover, key_press, focus |
| DOM | 3 | query_selector, get_attribute, set_attribute |
| Extraction | 9 | markdown, article (readability), PDF text, OCR, plain text, screenshots |
| Search | 1 | Web metasearch with OR query splitting |
| File Upload | 1 | Upload files to input[type=file] (resumes, documents, images) |
| Form Submit | 1 | React/XHR-aware form submission |
| Custom Dropdown | 1 | Non-native combobox interaction (click, type, select option) |
| Vault | 12 | Full credential lifecycle -- store, get, list, delete, rotate, TOTP, audit, domain approval |
| Cookies | 5 | get, set, save, load, import from Chrome profile |
| Cache | 10 | Full-text search, pin, tag, domain profiling, similarity, eviction |
| Intelligence | 6 | Entity queries, API discovery, site fingerprinting, page diff, auth detection, DNS |
| Entity Graph | 6 | Knowledge graph -- add, relate, merge, search, visualize (Mermaid) |
| Identity | 4 | Browser fingerprint profiles, TLS profile listing |
| Plugins | 4 | WASM plugin registry -- register, execute, list, remove |
| Telemetry | 2 | Browsing metrics, performance spans |
| Agent | 1 | Autonomous multi-step browsing task |
| Workflow | 4 | Record, replay, list reusable workflows |
| Time-Travel | 5 | Agent decision timeline -- branch, replay, diff, export |
| Task DAG | 7 | Parallel task orchestration with dependency resolution |
| MCTS Planning | 2 | Monte Carlo Tree Search action planning |
| Prefetch | 1 | Predict and pre-fetch next URLs |
| Swarm | 2 | Multi-URL parallel browsing |
| Embeddings | 2 | Semantic similarity search |
| Config | 1 | Engine capabilities and status |

## DOM Snapshots

Instead of raw HTML, Wraith produces compact agent-readable snapshots:

```
[Page: Search Results — https://example.com/search?q=engineer]

@e1  [search]  "" placeholder="Search"
@e2  [search]  "" placeholder="Location"
@e3  [button]  "Find Results"
@e4  [link]    "Senior Engineer — Company A" -> /view?id=123
@e5  [file]    "" (resume upload)
@e6  [link]    "Next >"

[6 interactive elements | ~40 tokens]
```

An agent reads this and responds: `ACTION: click @e4` or `ACTION: upload_file @e5 /path/to/resume.pdf`

## Form Automation

Wraith handles the full job application lifecycle:

- **Text fields** -- `browse_fill` by @ref ID
- **Custom dropdowns** -- `browse_custom_dropdown` opens, types to filter, selects matching option
- **File uploads** -- `browse_upload_file` reads from disk, injects via DataTransfer API
- **Form submission** -- `browse_submit_form` handles React/XHR forms, not just native POST
- **Realistic typing** -- `browse_type` with configurable keystroke delays

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
- Per-domain access controls with approval/revocation
- Automatic TOTP 2FA generation
- Chrome cookie import (reuse existing login sessions)
- Full audit trail of every credential access
- Secrets zeroized from memory immediately after use

## Intelligent Caching

Every page visited is cached, indexed, and searchable. Cache TTLs adapt automatically per domain based on observed content change frequency.

- SQLite + Tantivy full-text search
- Semantic page diffing (detects meaningful changes between visits)
- Cross-site entity resolution via knowledge graph
- Embedding store with cosine similarity search
- Pin important pages, tag for organized retrieval

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

---

## Support This Project

If you find this project useful, consider buying me a coffee! Your support helps me keep building and sharing open-source tools.

[![Donate via PayPal](https://img.shields.io/badge/Donate-PayPal-blue.svg?logo=paypal)](https://www.paypal.me/baal_hosting)

**PayPal:** [baal_hosting@live.com](https://paypal.me/baal_hosting)

Every donation, no matter how small, is greatly appreciated and motivates continued development. Thank you!
