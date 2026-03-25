#!/usr/bin/env bash
# bench_concurrent.sh — Concurrent session throughput benchmark for Wraith Browser
#
# Spawns increasing numbers of concurrent `wraith-browser navigate` processes,
# each fetching a test URL and extracting content. Measures total throughput
# (pages/second) and success rate at each concurrency level.
#
# Usage:
#   WRAITH_BIN=./target/release/wraith-browser ./benchmarks/bench_concurrent.sh
#
# Requirements:
#   - Linux with bash 4+, bc, awk, GNU parallel (optional but recommended)
#   - A built wraith-browser binary

set -euo pipefail

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------
WRAITH_BIN="${WRAITH_BIN:-./target/release/wraith-browser}"
CONCURRENCY_LEVELS="${CONCURRENCY_LEVELS:-10 50 100 500 1000}"
URL_FILE="${URL_FILE:-$(dirname "$0")/test_urls.txt}"
RESULTS_DIR="${RESULTS_DIR:-$(dirname "$0")/results}"
OUTPUT_FORMAT="${OUTPUT_FORMAT:-snapshot}"
TIMEOUT_SECS="${TIMEOUT_SECS:-30}"   # per-request timeout

mkdir -p "$RESULTS_DIR"

if [[ ! -x "$WRAITH_BIN" ]]; then
    echo "ERROR: wraith-browser binary not found at: $WRAITH_BIN"
    echo "       Build it first:  cargo build --release"
    exit 1
fi

if [[ ! -f "$URL_FILE" ]]; then
    echo "ERROR: URL file not found at: $URL_FILE"
    exit 1
fi

# Read URLs
mapfile -t URLS < <(grep -v '^\s*#' "$URL_FILE" | grep -v '^\s*$')
URL_COUNT=${#URLS[@]}

if [[ $URL_COUNT -eq 0 ]]; then
    echo "ERROR: No URLs in $URL_FILE"
    exit 1
fi

# ---------------------------------------------------------------------------
# Banner
# ---------------------------------------------------------------------------
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
REPORT_FILE="$RESULTS_DIR/concurrent_${TIMESTAMP}.txt"

echo "============================================================"
echo " Wraith Browser — Concurrent Session Throughput Benchmark"
echo "============================================================"
echo " Binary:        $WRAITH_BIN"
echo " Concurrency:   $CONCURRENCY_LEVELS"
echo " URLs:          $URL_COUNT (round-robin)"
echo " Timeout:       ${TIMEOUT_SECS}s per request"
echo " Started:       $(date -Iseconds)"
echo "============================================================"
echo ""

{
    echo "Wraith Browser Concurrent Throughput Benchmark — $(date -Iseconds)"
    echo "Binary: $WRAITH_BIN"
    echo "Concurrency levels: $CONCURRENCY_LEVELS"
    echo "URLs: $URL_COUNT | Timeout: ${TIMEOUT_SECS}s"
    echo ""
} > "$REPORT_FILE"

# ---------------------------------------------------------------------------
# Single-request worker function
# ---------------------------------------------------------------------------
# Writes "OK <elapsed_ms>" or "FAIL <elapsed_ms>" to stdout
run_one() {
    local url="$1"
    local start_ns end_ns elapsed_ms
    start_ns=$(date +%s%N)

    if timeout "$TIMEOUT_SECS" "$WRAITH_BIN" navigate "$url" --format "$OUTPUT_FORMAT" > /dev/null 2>&1; then
        end_ns=$(date +%s%N)
        elapsed_ms=$(echo "scale=2; ($end_ns - $start_ns) / 1000000" | bc)
        echo "OK $elapsed_ms"
    else
        end_ns=$(date +%s%N)
        elapsed_ms=$(echo "scale=2; ($end_ns - $start_ns) / 1000000" | bc)
        echo "FAIL $elapsed_ms"
    fi
}
export -f run_one
export WRAITH_BIN OUTPUT_FORMAT TIMEOUT_SECS

# ---------------------------------------------------------------------------
# Run benchmark at each concurrency level
# ---------------------------------------------------------------------------
printf "%-12s %-10s %-12s %-14s %-12s %-12s %-12s\n" \
    "Concurrency" "Success" "Failed" "Success Rate" "Total (s)" "Pages/sec" "Avg (ms)"
printf "%-12s %-10s %-12s %-14s %-12s %-12s %-12s\n" \
    "-----------" "-------" "------" "------------" "---------" "---------" "--------"

{
    printf "%-12s %-10s %-12s %-14s %-12s %-12s %-12s\n" \
        "Concurrency" "Success" "Failed" "Success Rate" "Total (s)" "Pages/sec" "Avg (ms)"
    printf "%-12s %-10s %-12s %-14s %-12s %-12s %-12s\n" \
        "-----------" "-------" "------" "------------" "---------" "---------" "--------"
} >> "$REPORT_FILE"

for level in $CONCURRENCY_LEVELS; do
    echo -ne "Running concurrency=$level ... "

    RESULT_FILE=$(mktemp)
    WALL_START=$(date +%s%N)

    # Build a list of URLs for this batch (round-robin from URL list)
    URL_BATCH=$(mktemp)
    for (( i=0; i<level; i++ )); do
        echo "${URLS[$((i % URL_COUNT))]}"
    done > "$URL_BATCH"

    # Check if GNU parallel is available for better concurrency control
    if command -v parallel &> /dev/null; then
        parallel -j "$level" --timeout "$TIMEOUT_SECS" \
            run_one {} < "$URL_BATCH" > "$RESULT_FILE" 2>/dev/null || true
    else
        # Fallback: bash background jobs with a simple job limiter
        PIDS=()
        WORKER_DIR=$(mktemp -d)

        while IFS= read -r url; do
            idx=${#PIDS[@]}
            (
                run_one "$url" > "$WORKER_DIR/result_${idx}" 2>/dev/null
            ) &
            PIDS+=($!)
        done < "$URL_BATCH"

        # Wait for all jobs
        for pid in "${PIDS[@]}"; do
            wait "$pid" 2>/dev/null || true
        done

        # Combine results
        cat "$WORKER_DIR"/result_* > "$RESULT_FILE" 2>/dev/null || true
        rm -rf "$WORKER_DIR"
    fi

    WALL_END=$(date +%s%N)
    WALL_SECS=$(echo "scale=3; ($WALL_END - $WALL_START) / 1000000000" | bc)

    rm -f "$URL_BATCH"

    # Parse results
    SUCCESS_COUNT=$(grep -c '^OK' "$RESULT_FILE" 2>/dev/null || echo 0)
    FAIL_COUNT=$(grep -c '^FAIL' "$RESULT_FILE" 2>/dev/null || echo 0)
    TOTAL=$((SUCCESS_COUNT + FAIL_COUNT))

    if (( TOTAL > 0 )); then
        SUCCESS_RATE=$(echo "scale=1; $SUCCESS_COUNT * 100 / $TOTAL" | bc)
    else
        SUCCESS_RATE="0"
    fi

    if [[ "$WALL_SECS" != "0" && "$WALL_SECS" != ".000" ]]; then
        PAGES_PER_SEC=$(echo "scale=1; $SUCCESS_COUNT / $WALL_SECS" | bc)
    else
        PAGES_PER_SEC="N/A"
    fi

    # Average latency of successful requests
    if (( SUCCESS_COUNT > 0 )); then
        AVG_MS=$(grep '^OK' "$RESULT_FILE" | awk '{s+=$2; c++} END {printf "%.1f", s/c}')
    else
        AVG_MS="N/A"
    fi

    printf "\r%-12s %-10s %-12s %-14s %-12s %-12s %-12s\n" \
        "$level" "$SUCCESS_COUNT" "$FAIL_COUNT" "${SUCCESS_RATE}%" "$WALL_SECS" "$PAGES_PER_SEC" "$AVG_MS"

    printf "%-12s %-10s %-12s %-14s %-12s %-12s %-12s\n" \
        "$level" "$SUCCESS_COUNT" "$FAIL_COUNT" "${SUCCESS_RATE}%" "$WALL_SECS" "$PAGES_PER_SEC" "$AVG_MS" >> "$REPORT_FILE"

    rm -f "$RESULT_FILE"

    # Brief pause between levels to let OS settle
    sleep 2
done

echo ""

# ---------------------------------------------------------------------------
# System info
# ---------------------------------------------------------------------------
echo "============================================================"
echo " System Info"
echo "============================================================"

{
    echo ""
    echo "=== System Info ==="
} >> "$REPORT_FILE"

for info_cmd in \
    "uname -a" \
    "nproc" \
    "free -h" \
    "cat /proc/cpuinfo | grep 'model name' | head -1"; do
    result=$(eval "$info_cmd" 2>/dev/null || echo "N/A")
    echo "  $info_cmd: $result"
    echo "  $info_cmd: $result" >> "$REPORT_FILE"
done

echo ""
echo "Report saved to: $REPORT_FILE"
echo "Done at $(date -Iseconds)"

# ---------------------------------------------------------------------------
# Notes
# ---------------------------------------------------------------------------
cat << 'NOTES'

============================================================
 Notes on Interpretation
============================================================
- These numbers measure CLI process launch + navigate + shutdown per request.
  The MCP server mode (single process, multiple sessions) will show better
  throughput because it avoids per-process startup cost.

- For MCP server throughput testing, use an MCP client to send concurrent
  browse_session_create + browse_navigate calls to a running server:

    wraith-browser serve --transport stdio

  Then fire concurrent tool calls and measure response times.

- Network latency to target sites dominates these numbers. For pure engine
  benchmarks, use a local HTTP server (e.g., python3 -m http.server).

- At high concurrency (500+), OS file descriptor limits may become a factor.
  Check: ulimit -n (should be >= 2 * concurrency level).
============================================================
NOTES
