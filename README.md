# Wraith Browser

**The AI-agent-first web browser — built in Rust for LLM control, not humans.**

[![License: AGPL-3.0](https://img.shields.io/badge/License-AGPL--3.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.75%2B-orange.svg)](https://www.rust-lang.org/)

---

Wraith is a native Rust web browser designed from the ground up for AI agents. Where traditional browsers optimize for human eyes, Wraith optimizes for token efficiency — delivering compressed DOM snapshots, structured content extraction, and a full MCP server that lets Claude Code, Cursor, or any AI agent browse the web autonomously.

**Think of it as the browser your AI agent deserves.**

## Why Wraith Exists

Every AI browser tool today is a wrapper around Playwright or Puppeteer — JavaScript runtimes bolted onto Chrome with human-first APIs. Wraith is different:

- **Native Rust + CDP** — Direct Chrome DevTools Protocol control. No Node.js. No JavaScript runtime overhead.
- **DOM Snapshots, Not HTML** — Instead of dumping 500KB of raw HTML, Wraith extracts a compact representation with `@ref` IDs for every interactive element. An LLM can read a 2000-element page in ~800 tokens.
- **Adaptive Knowledge Cache** — Every page the agent visits is cached, indexed, and searchable. The agent never asks the web the same question twice unless the answer is stale. TTLs adapt per-domain based on observed content change frequency.
- **Encrypted Credential Vault** — AES-256-GCM encrypted storage with argon2id key derivation. Agents can authenticate without leaking passwords into context windows.
- **MCP-Native** — Ships as an MCP server. `wraith serve --transport stdio` and your AI agent has a full browser.

## Architecture

```
                        ┌─────────────────────────────────┐
                        │         AI Agent (Claude, etc.)  │
                        └──────────────┬──────────────────┘
                                       │ MCP Protocol (stdio)
                        ┌──────────────▼──────────────────┐
                        │       wraith-mcp-server          │
                        │   14 tools: browse_navigate,     │
                        │   browse_click, browse_fill,     │
                        │   browse_extract, browse_search  │
                        └──────────────┬──────────────────┘
           ┌───────────────┬───────────┼───────────┬──────────────┐
           ▼               ▼           ▼           ▼              ▼
    ┌─────────────┐ ┌────────────┐ ┌────────┐ ┌────────┐ ┌──────────────┐
    │ browser-core│ │  content-  │ │ cache  │ │identity│ │   search     │
    │ (CDP/Chrome)│ │  extract   │ │(SQLite │ │(vault, │ │(DDG, Brave,  │
    │ snapshots,  │ │ readability│ │+Tantivy│ │ finger-│ │ local index) │
    │ actions     │ │ + markdown │ │+blobs) │ │ prints)│ │              │
    └─────────────┘ └────────────┘ └────────┘ └────────┘ └──────────────┘
```

### The 8 Crates

| Crate | Purpose |
|-------|---------|
| `wraith-browser-core` | Chrome DevTools Protocol control — sessions, tabs, DOM snapshots, action execution |
| `wraith-content-extract` | HTML noise stripping (lol_html), readability extraction, markdown conversion |
| `wraith-cache` | SQLite + Tantivy knowledge store with adaptive TTLs and blob storage |
| `wraith-identity` | AES-256-GCM credential vault, browser fingerprint capture/injection, auth flow detection |
| `wraith-search` | Web metasearch (DuckDuckGo HTML + Brave API) and local content index |
| `wraith-agent-loop` | Observe-think-act AI decision cycle with Claude and Ollama backends |
| `wraith-mcp-server` | MCP protocol server exposing 14 browser tools via stdio transport |
| `wraith-browser` (CLI) | Main binary with subcommands for all operations |

## Quick Start

### Build from Source

```bash
git clone https://github.com/suhteevah/wraith-browser.git
cd wraith-browser
cargo build --release
```

### Use as MCP Server with Claude Code

```bash
# Add Wraith as an MCP server
claude mcp add wraith-browser ./target/release/wraith-browser -- serve --transport stdio
```

Claude Code now has access to these tools:

| Tool | Description |
|------|-------------|
| `browse_navigate` | Navigate to a URL, return DOM snapshot with `@ref` IDs |
| `browse_click` | Click element by `@ref` ID |
| `browse_fill` | Fill form field by `@ref` ID |
| `browse_snapshot` | Get current page DOM snapshot |
| `browse_extract` | Extract page content as clean markdown |
| `browse_screenshot` | Capture PNG screenshot |
| `browse_search` | Web metasearch (DuckDuckGo + Brave) |
| `browse_eval_js` | Execute JavaScript on the page |
| `browse_tabs` | List open browser tabs |
| `browse_back` | Go back in history |
| `browse_key_press` | Press keyboard key |
| `browse_scroll` | Scroll the page |
| `browse_vault_store` | Store credential in encrypted vault |
| `browse_vault_get` | Retrieve credential from vault |

### CLI Usage

```bash
# Navigate and see DOM snapshot
wraith-browser navigate https://example.com

# Extract content as markdown
wraith-browser extract https://docs.rust-lang.org

# Search the web
wraith-browser search "rust async runtime benchmarks"

# Manage credentials
wraith-browser vault store --domain github.com --kind password --identity user@example.com
wraith-browser vault list

# Run an autonomous browsing task
wraith-browser task "Find the current price of Bitcoin on CoinGecko"
```

## Key Features

### DOM Snapshots — The Killer Feature

Instead of raw HTML, Wraith produces compact agent-readable snapshots:

```
[Page: Example Login — https://app.example.com/login]
[Type: login_form]

@e1 input[type=email] placeholder="Email address"
@e2 input[type=password] placeholder="Password"
@e3 button "Sign In"
@e4 a "Forgot password?" -> /reset
@e5 a "Create account" -> /signup

[5 interactive elements | ~12 tokens]
```

An AI agent reads this and responds: `ACTION: fill @e1 "user@example.com"`

### Adaptive Knowledge Cache

```
┌─────────────────────────────────────────────┐
│ Content Type        │ Default TTL │ Adaptive │
├─────────────────────┼─────────────┼──────────┤
│ Documentation/wikis │ 7 days      │ Yes      │
│ News articles       │ 1 hour      │ Yes      │
│ API docs            │ 24 hours    │ Yes      │
│ Social media        │ 15 minutes  │ Yes      │
│ Government/legal    │ 30 days     │ Yes      │
│ Search results      │ 6 hours     │ No       │
└─────────────────────────────────────────────┘
```

TTLs adjust automatically. If `docs.rust-lang.org` hasn't changed in 50 fetches, the TTL extends. If `news.ycombinator.com` changes every 10 minutes, it shrinks.

### Encrypted Credential Vault

- AES-256-GCM encryption with argon2id key derivation
- Credentials never appear in LLM context windows
- Domain-scoped approval system (credential X only works on domain Y)
- Full audit logging of every credential access
- TOTP 2FA generation for supported sites

### Content Extraction Pipeline

```
Raw HTML ──► lol_html (strip noise) ──► Readability (extract article)
         ──► Markdown conversion ──► Token-budgeted output
```

Strips scripts, ads, tracking pixels, hidden elements, and navigation chrome. Extracts the article body with links and images preserved. Converts to clean markdown optimized for LLM consumption.

## Configuration

### Environment Variables

| Variable | Description | Required |
|----------|-------------|----------|
| `BRAVE_SEARCH_API_KEY` | Brave Search API key for web search | No (falls back to DuckDuckGo) |
| `ANTHROPIC_API_KEY` | Anthropic API key for agent loop | Only for `task` command |
| `WRAITH_VAULT_PATH` | Custom path for credential vault | No (defaults to `~/.wraith/vault.db`) |
| `WRAITH_CACHE_PATH` | Custom path for knowledge cache | No (defaults to `~/.wraith/cache/`) |
| `RUST_LOG` | Logging level (`debug`, `info`, `warn`) | No (defaults to `info`) |

## Project Status

Wraith is in **active development**. All 8 crates compile, pass 71 tests, and produce a working release binary. The MCP server exposes all 14 tools with stub responses — wiring to live browser sessions is the next milestone.

### What Works Today
- Full CDP browser control (navigate, click, fill, screenshot, eval JS)
- DOM snapshot extraction with `@ref` IDs
- Content extraction pipeline (noise strip + readability + markdown)
- Encrypted credential vault with audit logging
- Knowledge cache with adaptive TTLs and full-text search
- MCP server with tool definitions and JSON Schema
- Agent loop with action parsing (Claude + Ollama backends)
- Web search (DuckDuckGo HTML + Brave API)
- CLI with all subcommands

### What's Next
- Wire MCP tool dispatch to live browser sessions
- Browser fingerprint injection (CDP overrides)
- Auth flow detection and auto-login
- Streaming MCP transport (SSE)
- Multi-tab session management
- Vision model support (screenshots in agent loop)

## License

**AGPL-3.0** — Wraith is free and open source software.

You can use Wraith freely for personal projects, open source work, academic research, and internal tools. If you modify Wraith and deploy it as a network service, you must release your modifications under the same license.

### Commercial Use

Companies that want to use Wraith in proprietary products without open-sourcing their codebase can obtain a **commercial license**. Contact [ridgecellrepair@gmail.com](mailto:ridgecellrepair@gmail.com) for licensing inquiries.

### Enterprise

**Wraith Enterprise** (coming soon) will include:
- Managed browser fleet with on-demand scaling
- Team credential vault with role-based access control
- Priority browser pool management
- Compliance dashboards and audit exports
- SLA-backed support
- SSO/SAML integration

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines. We welcome contributions of all sizes.

Key areas where help is needed:
- Additional search providers (Google, Bing)
- Browser fingerprint profile database
- Auth flow detection patterns
- Performance optimization
- Documentation and examples

## Acknowledgments

Built with:
- [chromiumoxide](https://github.com/nickel-ob/chromiumoxide) — Rust CDP client
- [lol_html](https://github.com/nickel-ob/lol-html) — Streaming HTML rewriter
- [Tantivy](https://github.com/quickwit-oss/tantivy) — Full-text search engine
- [rmcp](https://crates.io/crates/rmcp) — Rust MCP protocol implementation
- [scraper](https://github.com/causal-agent/scraper) — HTML parsing

---

**Wraith** — *the browser your AI agent deserves.*

Copyright (c) 2026 Matt Gates / Ridge Cell Repair LLC

---

---

## Support This Project

If you find this project useful, consider buying me a coffee! Your support helps me keep building and sharing open-source tools.

[![Donate via PayPal](https://img.shields.io/badge/Donate-PayPal-blue.svg?logo=paypal)](https://www.paypal.me/baal_hosting)

**PayPal:** [baal_hosting@live.com](https://paypal.me/baal_hosting)

Every donation, no matter how small, is greatly appreciated and motivates continued development. Thank you!
