#!/usr/bin/env bash
# bench_tokens.sh — Token savings benchmark for Wraith Browser
#
# Compares raw HTML size vs Wraith snapshot size for the same pages.
# Demonstrates the massive token savings that make Wraith ideal for
# AI agent use cases where every token costs money.
#
# Usage:
#   WRAITH_BIN=./target/release/wraith-browser ./benchmarks/bench_tokens.sh
#
# Requirements:
#   - Linux with bash 4+, bc, awk, curl
#   - A built wraith-browser binary (cargo build --release)
#   - Network access to test URLs

set -euo pipefail

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------
WRAITH_BIN="${WRAITH_BIN:-./target/release/wraith-browser}"
URL_FILE="${URL_FILE:-$(dirname "$0")/test_urls.txt}"
RESULTS_DIR="${RESULTS_DIR:-$(dirname "$0")/results}"
CURL_TIMEOUT="${CURL_TIMEOUT:-15}"
OUTPUT_FORMAT="${OUTPUT_FORMAT:-snapshot}"   # snapshot format for max compression

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
estimate_tokens() {
    # Rough estimate: 1 token ~ 4 characters (OpenAI's rule of thumb)
    local chars="$1"
    echo "scale=0; $chars / 4" | bc
}

format_number() {
    # Add comma separators to a number
    printf "%'d" "$1" 2>/dev/null || printf "%d" "$1"
}

# ---------------------------------------------------------------------------
# Run benchmark
# ---------------------------------------------------------------------------
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
REPORT_FILE="$RESULTS_DIR/tokens_${TIMESTAMP}.txt"

echo "============================================================"
echo " Wraith Browser — Token Savings Benchmark"
echo "============================================================"
echo " Binary:      $WRAITH_BIN"
echo " Format:      $OUTPUT_FORMAT"
echo " URLs:        ${#URLS[@]}"
echo " Started:     $(date -Iseconds)"
echo "============================================================"
echo ""

{
    echo "Wraith Browser Token Savings Benchmark — $(date -Iseconds)"
    echo "Binary: $WRAITH_BIN"
    echo "Format: $OUTPUT_FORMAT"
    echo ""
} > "$REPORT_FILE"

# Table header
HEADER=$(printf "%-50s | %12s | %12s | %12s | %12s | %8s" \
    "URL" "Raw HTML ch" "Raw ~tokens" "Snap ch" "Snap ~tokens" "Savings")
SEPARATOR=$(printf '%0.s-' {1..120})

echo "$HEADER"
echo "$SEPARATOR"

{
    echo "$HEADER"
    echo "$SEPARATOR"
} >> "$REPORT_FILE"

# Accumulators for summary
total_raw_chars=0
total_snap_chars=0
success_count=0
fail_count=0

for url in "${URLS[@]}"; do
    # 1. Fetch raw HTML with curl
    raw_html_file=$(mktemp)
    if ! curl -sL --max-time "$CURL_TIMEOUT" -o "$raw_html_file" "$url" 2>/dev/null; then
        echo "  SKIP: curl failed for $url"
        fail_count=$(( fail_count + 1 ))
        rm -f "$raw_html_file"
        continue
    fi

    raw_chars=$(wc -c < "$raw_html_file")
    rm -f "$raw_html_file"

    # Skip if curl returned empty response
    if [[ $raw_chars -eq 0 ]]; then
        echo "  SKIP: empty response for $url"
        fail_count=$(( fail_count + 1 ))
        continue
    fi

    raw_tokens=$(estimate_tokens "$raw_chars")

    # 2. Get Wraith snapshot
    snap_file=$(mktemp)
    if ! "$WRAITH_BIN" navigate "$url" --format "$OUTPUT_FORMAT" > "$snap_file" 2>/dev/null; then
        echo "  SKIP: wraith failed for $url"
        fail_count=$(( fail_count + 1 ))
        rm -f "$snap_file"
        continue
    fi

    snap_chars=$(wc -c < "$snap_file")
    rm -f "$snap_file"

    # Skip if snapshot is empty
    if [[ $snap_chars -eq 0 ]]; then
        echo "  SKIP: empty snapshot for $url"
        fail_count=$(( fail_count + 1 ))
        continue
    fi

    snap_tokens=$(estimate_tokens "$snap_chars")

    # 3. Calculate savings
    if [[ $raw_chars -gt 0 ]]; then
        savings=$(echo "scale=1; (1 - $snap_chars / $raw_chars) * 100" | bc)
    else
        savings="0.0"
    fi

    # Accumulate
    total_raw_chars=$(( total_raw_chars + raw_chars ))
    total_snap_chars=$(( total_snap_chars + snap_chars ))
    success_count=$(( success_count + 1 ))

    # Truncate URL for display
    display_url="$url"
    if [[ ${#display_url} -gt 48 ]]; then
        display_url="${display_url:0:45}..."
    fi

    line=$(printf "%-50s | %12d | %12d | %12d | %12d | %7s%%" \
        "$display_url" "$raw_chars" "$raw_tokens" "$snap_chars" "$snap_tokens" "$savings")
    echo "$line"
    echo "$line" >> "$REPORT_FILE"
done

echo "$SEPARATOR"
echo "$SEPARATOR" >> "$REPORT_FILE"

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
echo ""
echo "============================================================"
echo " SUMMARY"
echo "============================================================"

{
    echo ""
    echo "============================================================"
    echo "SUMMARY"
    echo "============================================================"
} >> "$REPORT_FILE"

echo "  Pages tested:    $success_count successful, $fail_count failed"

if [[ $success_count -gt 0 ]]; then
    total_raw_tokens=$(estimate_tokens "$total_raw_chars")
    total_snap_tokens=$(estimate_tokens "$total_snap_chars")
    total_savings=$(echo "scale=1; (1 - $total_snap_chars / $total_raw_chars) * 100" | bc)

    avg_raw_chars=$(( total_raw_chars / success_count ))
    avg_snap_chars=$(( total_snap_chars / success_count ))
    avg_raw_tokens=$(estimate_tokens "$avg_raw_chars")
    avg_snap_tokens=$(estimate_tokens "$avg_snap_chars")

    echo ""
    echo "  Total raw HTML:     $(format_number $total_raw_chars) chars  (~$(format_number $total_raw_tokens) tokens)"
    echo "  Total snapshots:    $(format_number $total_snap_chars) chars  (~$(format_number $total_snap_tokens) tokens)"
    echo "  Overall savings:    ${total_savings}%"
    echo ""
    echo "  Avg per page:"
    echo "    Raw HTML:         $(format_number $avg_raw_chars) chars  (~$(format_number $avg_raw_tokens) tokens)"
    echo "    Wraith snapshot:  $(format_number $avg_snap_chars) chars  (~$(format_number $avg_snap_tokens) tokens)"
    echo ""
    echo "  -------------------------------------------------------"
    echo "  KEY INSIGHT: Raw HTML consumes $(format_number $total_raw_tokens) tokens."
    echo "  Wraith snapshots use only $(format_number $total_snap_tokens) tokens."
    echo "  That's a ${total_savings}% reduction in token costs."
    echo "  -------------------------------------------------------"
    echo ""
    echo "  At \$3/M input tokens (GPT-4 class), processing these"
    echo "  $success_count pages raw costs ~\$$(echo "scale=4; $total_raw_tokens * 3 / 1000000" | bc)."
    echo "  With Wraith snapshots: ~\$$(echo "scale=4; $total_snap_tokens * 3 / 1000000" | bc)."

    {
        echo "  Pages: $success_count successful, $fail_count failed"
        echo "  Total raw HTML:    $total_raw_chars chars (~$total_raw_tokens tokens)"
        echo "  Total snapshots:   $total_snap_chars chars (~$total_snap_tokens tokens)"
        echo "  Overall savings:   ${total_savings}%"
        echo ""
        echo "  Avg per page:"
        echo "    Raw HTML:        $avg_raw_chars chars (~$avg_raw_tokens tokens)"
        echo "    Wraith snapshot: $avg_snap_chars chars (~$avg_snap_tokens tokens)"
    } >> "$REPORT_FILE"
fi

echo ""
echo "Report saved to: $REPORT_FILE"
echo "Done at $(date -Iseconds)"
