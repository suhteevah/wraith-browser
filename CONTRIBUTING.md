# Contributing to Wraith Browser

Thanks for your interest! This project is in early development.

## Getting Started

1. Install Rust 1.75+ via [rustup](https://rustup.rs/)
2. Install Chrome or Chromium (needed for browser-core)
3. Clone and build: `cargo build`
4. Run tests: `cargo test`

## Architecture

See README.md for the crate layout. 8 crates, each with its own responsibility:

- **browser-core**: CDP abstraction. Browser control, DOM snapshots, actions.
- **content-extract**: HTML→Markdown pipeline. Readability + conversion.
- **cache**: SQLite + Tantivy knowledge store. Adaptive TTLs, compressed blobs.
- **identity**: Encrypted credential vault, fingerprint spoofing, auth flows.
- **search-engine**: Metasearch and local indexing.
- **agent-loop**: The AI decision cycle. Needs LLM integration work.
- **mcp-server**: MCP protocol tools for Claude Code/Cursor.
- **cli**: The main binary. Wires everything together.

## Code Style

- Use `tracing` for ALL logging (never `println!` in library code)
- Every public function gets `#[instrument]`
- Return `Result<T, E>` — no `.unwrap()` in production paths
- Secrets use `SecretString` / `SecretVec` — never log, never serialize
- Run `cargo clippy` before submitting

## License

By contributing, you agree your code is licensed under AGPL-3.0.
