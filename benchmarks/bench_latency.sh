#!/usr/bin/env bash
# bench_latency.sh — Page fetch latency benchmark for Wraith Browser
#
# Measures wall-clock time for `wraith-browser navigate <url>` across
# repeated iterations and reports min/avg/p50/p95/p99/max.
#
# Usage:
#   WRAITH_BIN=./target/release/wraith-browser ./benchmarks/bench_latency.sh
#   WRAITH_BIN=./target/release/wraith-browser ITERATIONS=50 ./benchmarks/bench_latency.sh
#
# Requirements:
#   - Linux with bash 4+, bc, sort, awk
#   - A built wraith-browser binary (cargo build --release)
#   - Network access to test URLs

set -euo pipefail

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------
WRAITH_BIN="${WRAITH_BIN:-./target/release/wraith-browser}"
ITERATIONS="${ITERATIONS:-100}"
URL_FILE="${URL_FILE:-$(dirname "$0")/test_urls.txt}"
OUTPUT_FORMAT="${OUTPUT_FORMAT:-snapshot}"   # snapshot | markdown | json
WARMUP="${WARMUP:-3}"                       # warmup runs per URL (discarded)
RESULTS_DIR="${RESULTS_DIR:-$(dirname "$0")/results}"

mkdir -p "$RESULTS_DIR"

# ---------------------------------------------------------------------------
# Validation
# ---------------------------------------------------------------------------
if [[ ! -x "$WRAITH_BIN" ]]; then
    echo "ERROR: wraith-browser binary not found or not executable at: $WRAITH_BIN"
    echo "       Build it first:  cargo build --release"
    exit 1
fi

if [[ ! -f "$URL_FILE" ]]; then
    echo "ERROR: URL file not found at: $URL_FILE"
    exit 1
fi

# Read URLs, skip comments and blanks
mapfile -t URLS < <(grep -v '^\s*#' "$URL_FILE" | grep -v '^\s*$')

if [[ ${#URLS[@]} -eq 0 ]]; then
    echo "ERROR: No URLs found in $URL_FILE"
    exit 1
fi

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------
percentile() {
    # Usage: percentile <file> <p>  (p is 0-100)
    local file="$1" p="$2"
    local count
    count=$(wc -l < "$file")
    local index
    index=$(echo "scale=0; ($p / 100) * ($count - 1) + 1" | bc)
    index=$(printf "%.0f" "$index")
    if (( index < 1 )); then index=1; fi
    if (( index > count )); then index=$count; fi
    sed -n "${index}p" "$file"
}

stats() {
    local file="$1"
    local sorted
    sorted=$(mktemp)
    sort -n "$file" > "$sorted"

    local count min max avg p50 p95 p99 sum
    count=$(wc -l < "$sorted")
    min=$(head -1 "$sorted")
    max=$(tail -1 "$sorted")
    sum=$(awk '{s+=$1} END {print s}' "$sorted")
    avg=$(echo "scale=2; $sum / $count" | bc)
    p50=$(percentile "$sorted" 50)
    p95=$(percentile "$sorted" 95)
    p99=$(percentile "$sorted" 99)

    echo "$count $min $avg $p50 $p95 $p99 $max"
    rm -f "$sorted"
}

# ---------------------------------------------------------------------------
# Run benchmark
# ---------------------------------------------------------------------------
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
REPORT_FILE="$RESULTS_DIR/latency_${TIMESTAMP}.txt"

echo "============================================================"
echo " Wraith Browser — Page Fetch Latency Benchmark"
echo "============================================================"
echo " Binary:      $WRAITH_BIN"
echo " Iterations:  $ITERATIONS per URL"
echo " Warmup:      $WARMUP per URL"
echo " Format:      $OUTPUT_FORMAT"
echo " URLs:        ${#URLS[@]}"
echo " Started:     $(date -Iseconds)"
echo "============================================================"
echo ""

{
    echo "Wraith Browser Latency Benchmark — $(date -Iseconds)"
    echo "Binary: $WRAITH_BIN"
    echo "Iterations: $ITERATIONS | Warmup: $WARMUP | Format: $OUTPUT_FORMAT"
    echo ""
} > "$REPORT_FILE"

ALL_TIMES=$(mktemp)

for url in "${URLS[@]}"; do
    echo "--- $url ---"

    # Warmup (discard results)
    for (( w=1; w<=WARMUP; w++ )); do
        "$WRAITH_BIN" navigate "$url" --format "$OUTPUT_FORMAT" > /dev/null 2>&1 || true
    done

    TIMES_FILE=$(mktemp)

    for (( i=1; i<=ITERATIONS; i++ )); do
        # Measure wall-clock time in milliseconds
        start_ns=$(date +%s%N)
        "$WRAITH_BIN" navigate "$url" --format "$OUTPUT_FORMAT" > /dev/null 2>&1
        exit_code=$?
        end_ns=$(date +%s%N)

        elapsed_ms=$(echo "scale=2; ($end_ns - $start_ns) / 1000000" | bc)

        if [[ $exit_code -eq 0 ]]; then
            echo "$elapsed_ms" >> "$TIMES_FILE"
            echo "$elapsed_ms" >> "$ALL_TIMES"
        fi

        # Progress indicator every 10 iterations
        if (( i % 10 == 0 )); then
            echo -ne "  $i/$ITERATIONS completed\r"
        fi
    done

    echo ""

    success_count=$(wc -l < "$TIMES_FILE")
    fail_count=$(( ITERATIONS - success_count ))

    if [[ $success_count -gt 0 ]]; then
        read -r count min avg p50 p95 p99 max <<< "$(stats "$TIMES_FILE")"
        printf "  Results: %d/%d successful\n" "$success_count" "$ITERATIONS"
        printf "  min=%.2fms  avg=%.2fms  p50=%.2fms  p95=%.2fms  p99=%.2fms  max=%.2fms\n\n" \
            "$min" "$avg" "$p50" "$p95" "$p99" "$max"

        {
            printf "URL: %s\n" "$url"
            printf "  Success: %d/%d  Failures: %d\n" "$success_count" "$ITERATIONS" "$fail_count"
            printf "  min=%.2fms  avg=%.2fms  p50=%.2fms  p95=%.2fms  p99=%.2fms  max=%.2fms\n\n" \
                "$min" "$avg" "$p50" "$p95" "$p99" "$max"
        } >> "$REPORT_FILE"
    else
        echo "  All $ITERATIONS iterations failed for this URL."
        echo "URL: $url — ALL FAILED" >> "$REPORT_FILE"
    fi

    rm -f "$TIMES_FILE"
done

# ---------------------------------------------------------------------------
# Aggregate
# ---------------------------------------------------------------------------
echo "============================================================"
echo " AGGREGATE (all URLs combined)"
echo "============================================================"

total_success=$(wc -l < "$ALL_TIMES")
if [[ $total_success -gt 0 ]]; then
    read -r count min avg p50 p95 p99 max <<< "$(stats "$ALL_TIMES")"
    printf "  Total fetches: %d\n" "$count"
    printf "  min=%.2fms  avg=%.2fms  p50=%.2fms  p95=%.2fms  p99=%.2fms  max=%.2fms\n" \
        "$min" "$avg" "$p50" "$p95" "$p99" "$max"

    {
        echo "============================================================"
        echo "AGGREGATE"
        printf "  Total: %d\n" "$count"
        printf "  min=%.2fms  avg=%.2fms  p50=%.2fms  p95=%.2fms  p99=%.2fms  max=%.2fms\n" \
            "$min" "$avg" "$p50" "$p95" "$p99" "$max"
    } >> "$REPORT_FILE"
else
    echo "  No successful fetches recorded."
fi

rm -f "$ALL_TIMES"

echo ""
echo "Report saved to: $REPORT_FILE"
echo "Done at $(date -Iseconds)"
