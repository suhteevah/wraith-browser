# Wraith Browser Performance Benchmarks

Reproducible benchmarks for validating Wraith Browser's performance claims.

## Claims Under Test

| Claim | Target | Benchmark |
|-------|--------|-----------|
| Page fetch latency | ~50ms per page | `bench_latency.sh` |
| Session memory | 8-12 MB per session | `bench_memory.sh` |
| Concurrent sessions | High session counts | `bench_concurrent.sh` |
| Token savings | 90-95%+ vs raw HTML | `bench_tokens.sh` |

## Quick Start

```bash
# 1. Build wraith-browser in release mode
cargo build --release

# 2. Run all benchmarks
export WRAITH_BIN=./target/release/wraith-browser
./benchmarks/bench_latency.sh
./benchmarks/bench_memory.sh
./benchmarks/bench_concurrent.sh
./benchmarks/bench_tokens.sh
```

Results are written to `benchmarks/results/` with timestamps.

## Benchmarks

### bench_latency.sh

Measures wall-clock time for `wraith-browser navigate <url>` across 100 iterations per URL. Reports min, avg, p50, p95, p99, and max latency.

**What it tests:** Full CLI round-trip — process startup, engine initialization, HTTP fetch, DOM parsing, snapshot generation, and shutdown. The ~50ms claim refers to the engine's internal page fetch time; CLI overhead adds process startup cost on top.

**Environment variables:**
- `WRAITH_BIN` — Path to binary (default: `./target/release/wraith-browser`)
- `ITERATIONS` — Runs per URL (default: `100`)
- `WARMUP` — Warmup runs discarded (default: `3`)
- `URL_FILE` — Path to URL list (default: `benchmarks/test_urls.txt`)
- `OUTPUT_FORMAT` — `snapshot`, `markdown`, or `json` (default: `snapshot`)

### bench_memory.sh

Measures RSS memory at increasing session counts. Uses two methods:

1. **MCP server mode** — Starts the MCP server process and sends `browse_session_create` calls, measuring RSS of the single server process. This gives the true per-session memory overhead.

2. **CLI process mode** — Launches N concurrent `wraith-browser navigate` processes and sums their RSS. This includes per-process overhead and will report higher numbers than MCP mode.

**Environment variables:**
- `WRAITH_BIN` — Path to binary
- `SESSION_COUNTS` — Space-separated list (default: `"10 50 100 500"`)
- `TEST_URL` — URL to navigate (default: `http://example.com`)
- `SETTLE_TIME` — Seconds to wait before measuring (default: `2`)

### bench_concurrent.sh

Spawns increasing numbers of concurrent requests and measures throughput (pages/second) and success rate.

**Environment variables:**
- `WRAITH_BIN` — Path to binary
- `CONCURRENCY_LEVELS` — Space-separated list (default: `"10 50 100 500 1000"`)
- `TIMEOUT_SECS` — Per-request timeout (default: `30`)
- `URL_FILE` — Path to URL list

If GNU `parallel` is installed, it is used for better concurrency control. Otherwise, bash background jobs are used.

### bench_tokens.sh

Compares raw HTML size (fetched via `curl`) against Wraith snapshot size for the same pages. Calculates character counts, estimated token counts (chars/4), and compression ratios to quantify token savings.

**What it tests:** The core value proposition for AI agent use cases — how much Wraith's compact `@ref` snapshot representation reduces the data an LLM needs to consume compared to raw HTML. Raw HTML for a typical page is 50-100K tokens; Wraith snapshots compress this to 2-5K tokens, yielding 90-95%+ savings.

**Output:** A table showing per-URL comparisons plus aggregate statistics including estimated dollar savings at typical LLM pricing.

```
URL                                                |  Raw HTML ch |  Raw ~tokens |      Snap ch | Snap ~tokens | Savings
http://example.com                                 |         1256 |          314 |          187 |           46 |   85.1%
https://books.toscrape.com                         |        51342 |        12835 |         3214 |          803 |   93.7%
```

**Environment variables:**
- `WRAITH_BIN` — Path to binary (default: `./target/release/wraith-browser`)
- `URL_FILE` — Path to URL list (default: `benchmarks/test_urls.txt`)
- `OUTPUT_FORMAT` — Snapshot format (default: `snapshot`)
- `CURL_TIMEOUT` — Timeout for raw HTML fetch in seconds (default: `15`)

## Token Savings — A Key Metric

Token savings is one of Wraith Browser's most important selling points for AI agent use cases. Every token sent to an LLM costs money, and raw HTML is extraordinarily wasteful:

- **Raw HTML** includes CSS classes, inline styles, script tags, SVG paths, tracking pixels, metadata, and other markup that is irrelevant to understanding page content and structure. A simple page like example.com is ~1.2KB of HTML. A real-world page is routinely 50-200KB — that's **12,000-50,000 tokens** per page.

- **Wraith snapshots** use a compact `@ref` representation that preserves only the semantic structure and interactive elements an AI agent needs: visible text, links, buttons, form fields, and their identifiers. The same pages compress to 1-5KB — typically **250-1,250 tokens**.

This means:

| Page type | Raw HTML tokens | Wraith tokens | Savings |
|-----------|----------------|---------------|---------|
| Simple static (example.com) | ~300 | ~50 | ~85% |
| Medium docs/blog | ~15,000 | ~1,000 | ~93% |
| Heavy e-commerce/news | ~50,000 | ~2,500 | ~95% |

**Why this matters financially:**

At $3/M input tokens (GPT-4 class pricing), an agent browsing 1,000 pages:
- Raw HTML: ~30M tokens = **$90**
- Wraith snapshots: ~2M tokens = **$6**
- **Savings: $84 per 1,000 pages (93%)**

At scale — crawling, monitoring, or agentic workflows hitting thousands of pages per day — the difference between raw HTML and Wraith snapshots is the difference between a viable product and a cost-prohibitive one.

Run `bench_tokens.sh` against your own target URLs to get concrete numbers for your use case.

## Test URLs

`test_urls.txt` contains public URLs grouped by complexity:

- **Static:** example.com, httpbin.org/html
- **Lightweight dynamic:** books.toscrape.com, quotes.toscrape.com
- **Medium:** httpbin.org forms and link pages
- **Heavier:** the-internet.herokuapp.com pages

These sites are chosen because they tolerate modest benchmark traffic without rate limiting. Do not increase iteration counts dramatically against public sites.

## Hardware Requirements

Minimum for meaningful results:
- 4+ CPU cores (concurrent benchmarks need headroom)
- 8 GB RAM (memory benchmark at 500 sessions needs ~6 GB)
- Stable network connection (latency benchmarks are network-sensitive)
- Linux (scripts use `/proc`, `ps -o rss`, `date +%s%N`)

For comparable results across runs:
- Close other heavy applications
- Use a wired connection (not Wi-Fi)
- Run multiple times and compare across runs

## Methodology

### What these benchmarks measure

The CLI benchmarks measure **end-to-end wall-clock time** including:
- Process startup and Tokio runtime initialization
- Engine creation (Sevro native engine)
- HTTP fetch, TLS handshake, response parsing
- DOM construction and snapshot generation
- Process shutdown

The ~50ms claim is for the **engine-internal** page fetch time (HTTP + parse), not the full CLI round-trip. To isolate engine time from process overhead, compare:
- CLI round-trip time (what these scripts measure)
- Single process with multiple sequential navigations (subtract first-run startup)

### What these benchmarks do NOT measure

- JavaScript execution performance (QuickJS is used for JS, not V8)
- Rendering or screenshot performance
- WebSocket or streaming performance
- Performance under proxy or FlareSolverr configurations

### Honest reporting

These benchmarks are designed to establish baselines, not cherry-pick favorable numbers. Key principles:

1. **No warmup hiding** — Warmup runs are counted separately and clearly labeled
2. **Percentiles over averages** — p95 and p99 show tail latency, not just happy path
3. **Failure counting** — Failed requests are tracked and reported, not silently dropped
4. **Network included** — Real network latency is included; we don't mock HTTP
5. **System info recorded** — CPU, RAM, and OS are logged with every run

### Local server testing

For benchmarks that isolate engine performance from network latency, run a local HTTP server:

```bash
# Serve a static page locally
echo '<html><body><h1>Benchmark</h1><p>Test content.</p></body></html>' > /tmp/bench.html
cd /tmp && python3 -m http.server 8888 &

# Point benchmarks at localhost
TEST_URL=http://localhost:8888/bench.html ./benchmarks/bench_latency.sh
```

## Output Format

Each benchmark writes a timestamped report to `benchmarks/results/`:

```
results/
  latency_20240315_143022.txt
  memory_20240315_143055.txt
  concurrent_20240315_143120.txt
```

Reports are plain text, suitable for diffing across runs or pasting into issues.
