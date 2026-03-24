# Wraith Browser

**The AI-agent-first web browser -- built in Rust, designed for LLM control.**

[![License: AGPL-3.0](https://img.shields.io/badge/License-AGPL--3.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.75%2B-orange.svg)](https://www.rust-lang.org/)
[![MCP Tools](https://img.shields.io/badge/MCP%20tools-130-blue.svg)]()

---

Wraith is a native Rust browser engine purpose-built for AI agents. No Chrome dependency. No Node.js. Ships as a single ~15MB binary or MCP server with 130 tools -- every capability accessible via MCP calls. The stack is built on Mozilla's html5ever parser, rquickjs for JavaScript execution, and reqwest for HTTP. The user never touches this browser directly; the AI agent has full admin control.

**Full documentation:** [wraith-browser.vercel.app](https://wraith-browser.vercel.app)

## Why Wraith

| | Wraith | Traditional Automation |
|---|---|---|
| Chrome required | No | Yes (300MB+) |
| Memory per session | 5-50 MB | 300-500 MB |
| Page fetch (static) | ~50ms | 1-3 seconds |
| Binary size | ~15 MB | ~300 MB + runtime |
| Startup time | <100ms | 2-5 seconds |
| Concurrent sessions (16GB) | 50-100+ | 6-8 |
| MCP native | Yes (130 tools) | No |
| Knowledge graph | Built-in | Not available |
| Workflow record/replay | Built-in | Not available |

## Quick Start

### Build

```bash
git clone https://github.com/suhteevah/wraith-browser.git
cd wraith-browser
cargo build --release
```

### Connect to Claude Code

```bash
claude mcp add wraith ./target/release/wraith-browser -- serve --transport stdio
```

Your AI agent immediately gains 130 browser tools -- full admin control with zero CLI interaction.

### CLI

```bash
# Navigate and see interactive elements
wraith-browser navigate https://example.com

# Extract content as clean markdown
wraith-browser extract https://example.com/docs --max-tokens 4000

# Search the web (supports OR queries)
wraith-browser search "QA engineer OR SDET remote"

# Autonomous browsing task
ANTHROPIC_API_KEY=sk-... wraith-browser task "Find remote Rust jobs"

# Manage encrypted credentials
wraith-browser vault store --domain example.com --kind password --identity user@example.com
```

### Environment Variables

| Variable | Purpose |
|----------|---------|
| `WRAITH_FLARESOLVERR` | URL for external challenge-handling proxy |
| `WRAITH_PROXY` | Primary HTTP/SOCKS5 proxy URL |
| `WRAITH_FALLBACK_PROXY` | Fallback proxy for IP-blocked sites |
| `ANTHROPIC_API_KEY` | Required for `browse_task` autonomous agent |
| `BRAVE_SEARCH_API_KEY` | Optional Brave Search provider |
| `TWOCAPTCHA_API_KEY` | Required for `browse_solve_captcha` CAPTCHA integration |

---

## Feature Highlights

### 130 MCP Tools

Full browser automation via MCP protocol -- navigation, form filling, file uploads, credential management, caching, knowledge graphs, workflow recording, and more.

See the [complete tool reference](https://wraith-browser.vercel.app) for parameters and usage.

### html5ever-based Parsing

Built on Mozilla's html5ever parser with a full DOM tree, QuickJS JavaScript runtime, and DOM bridge. Executes inline scripts, handles SPA hydration, and supports React-compatible form filling.

### ATS-Aware Form Submission

Detects common applicant tracking systems and submits via their native APIs (multipart, form-urlencoded, JSON, or GraphQL) instead of fragile DOM scripting.

### Credential Vault

AES-256-GCM encrypted storage with Argon2id key derivation. Per-domain access controls, TOTP generation, Chrome cookie import, and full audit trail. Secrets are zeroized from memory after use.

### Intelligent Caching

SQLite-backed cache with full-text search, semantic page diffing, adaptive TTLs, and cross-site entity resolution via a built-in knowledge graph.

### Agent Intelligence

- **MCTS Planning** -- Monte Carlo Tree Search for optimal action selection
- **Task DAGs** -- parallel task graphs with dependency tracking
- **Time-Travel Debugging** -- replay, branch, and diff decision timelines
- **Workflow Record/Replay** -- capture and replay sessions with variable substitution
- **Swarm Browsing** -- visit multiple URLs in parallel

### Plugin System

WASM plugins (wasmtime), Rhai userscripts with navigation triggers, and a vision ML pipeline (ort/ONNX) for UI element detection.

---

## Architecture

10 Rust crates:

| Crate | Purpose |
|-------|---------|
| `browser-core` | Unified engine trait, ATS detection, network layer, swarm, plugins |
| `sevro-headless` | Headless engine -- HTTP, html5ever-based DOM parsing, QuickJS JS runtime, SPA hydration |
| `agent-loop` | LLM agent cycle -- MCTS planning, time-travel, workflows, task DAGs |
| `cache` | SQLite knowledge store, full-text search, embeddings, entity graph |
| `content-extract` | Readability extraction, markdown conversion, OCR, PDF text extraction |
| `identity` | AES-256-GCM encrypted credential vault, browser profiles, TOTP |
| `mcp-server` | MCP protocol server (130 tools, stdio transport) |
| `search-engine` | Metasearch (multiple providers), OR query splitting |
| `scripting` | Rhai sandboxed scripting engine (userscripts with navigation triggers) |
| `cli` | Binary with subcommands (`navigate`, `extract`, `search`, `task`, `vault`) |

---

## License

**AGPL-3.0** -- free and open source.

Use freely for personal projects, open source, research, and internal tools. If you modify Wraith and deploy it as a network service, modifications must be released under the same license.

### Commercial License

Companies that want to embed Wraith in proprietary products without open-source obligations can obtain a commercial license. Contact [ridgecellrepair@gmail.com](mailto:ridgecellrepair@gmail.com).

Contact us for enterprise features.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines. Key areas:

- ATS platform integrations (new API adapters)
- Search provider integrations
- Auth flow detection patterns
- Documentation and examples

## Acknowledgments

Built with [scraper](https://github.com/causal-agent/scraper), [rquickjs](https://crates.io/crates/rquickjs), [Tantivy](https://github.com/quickwit-oss/tantivy), [rmcp](https://crates.io/crates/rmcp), [ort](https://crates.io/crates/ort), [wasmtime](https://crates.io/crates/wasmtime), [petgraph](https://crates.io/crates/petgraph), and [reqwest](https://crates.io/crates/reqwest).

---

**Wraith** -- *the browser your AI agent deserves.*

Copyright (c) 2026 Matt Gates / Ridge Cell Repair LLC
